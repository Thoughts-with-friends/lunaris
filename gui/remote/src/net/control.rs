//! Requests that must arrive whole: the client's save import / export and
//! savestate slots, carried out by the host on the streamed console.
//!
//! ```text
//!  client                                   host (same port as the stream, TCP)
//!  RemoteClient::request(Request) ──────→   ControlServer thread
//!                                            └ queue ── RemoteHost::take_request
//!                                                       (the streamed console's
//!                                                        thread does it, then
//!                                                        Pending::reply)
//!  Ok(bytes) / Err(message)       ←──────   reply
//! ```
//!
//! The stream itself is UDP and may lose anything; a save may not, so these
//! go over TCP. One request per connection, only from the session's client.
//!
//! Frame: `tag u8 | length u32 LE | payload`. Requests: 1 import save
//! (payload: the file), 2 export save, 3 save state, 4 load state (payload:
//! the slot). Replies: 0 ok (payload: the save for an export), 1 error
//! (payload: UTF-8 message).

use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// Larger than any DS save (8 MiB NAND).
const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
/// How long the host may take to answer (an import reboots the console).
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the client tries to reach the host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the listener checks for shutdown.
const POLL: Duration = Duration::from_millis(50);

const OK: u8 = 0;
const ERR: u8 = 1;

/// What the client asks the host to do on the streamed console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Replace its save with this file (`.sav` / `.dsv`); the console
    /// restarts on it.
    ImportSave(Vec<u8>),
    /// Send its save (written out first).
    ExportSave,
    /// Save / load savestate slot `n` (kept on the host).
    SaveState(u8),
    LoadState(u8),
}

impl Request {
    const fn tag(&self) -> u8 {
        match self {
            Self::ImportSave(_) => 1,
            Self::ExportSave => 2,
            Self::SaveState(_) => 3,
            Self::LoadState(_) => 4,
        }
    }

    fn payload(&self) -> &[u8] {
        match self {
            Self::ImportSave(data) => data,
            Self::ExportSave => &[],
            Self::SaveState(slot) | Self::LoadState(slot) => std::slice::from_ref(slot),
        }
    }

    fn decode(tag: u8, payload: Vec<u8>) -> Option<Self> {
        let slot = || payload.first().copied();
        match tag {
            1 => Some(Self::ImportSave(payload)),
            2 => Some(Self::ExportSave),
            3 => slot().map(Self::SaveState),
            4 => slot().map(Self::LoadState),
            _ => None,
        }
    }
}

/// A request waiting for the host's answer.
pub struct Pending {
    pub request: Request,
    reply: mpsc::Sender<Result<Vec<u8>, String>>,
}

impl Pending {
    /// Answers the client: `Ok` with the save for an export (empty
    /// otherwise), or why it failed.
    pub fn reply(self, result: Result<Vec<u8>, String>) {
        let _ = self.reply.send(result);
    }
}

fn write_frame(w: &mut impl Write, tag: u8, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
    w.write_all(&[tag])?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0_u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "payload too large"));
    }
    let mut payload = vec![0; len];
    r.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

/// The host's listener and the requests it has taken in.
pub struct ControlServer {
    queue: Arc<Mutex<VecDeque<Pending>>>,
    shutdown: Arc<AtomicBool>,
}

impl ControlServer {
    /// Listens on `bind` for requests from `peer` only.
    ///
    /// # Errors
    /// If the port cannot be bound.
    pub fn start(bind: SocketAddr, peer: IpAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(bind)?;
        listener.set_nonblocking(true)?;
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (q, stop) = (Arc::clone(&queue), Arc::clone(&shutdown));
        std::thread::Builder::new().name("remote-control".to_owned()).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, from)) if from.ip() == peer || peer.is_loopback() => {
                        let q = Arc::clone(&q);
                        std::thread::spawn(move || {
                            if let Err(e) = serve(stream, &q) {
                                log::warn!("Remote Desktop request: {e}");
                            }
                        });
                    }
                    Ok((_, from)) => log::warn!("Remote Desktop: request from {from} refused"),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
                    Err(e) => {
                        log::warn!("Remote Desktop control: {e}");
                        std::thread::sleep(POLL);
                    }
                }
            }
        })?;
        Ok(Self { queue, shutdown })
    }

    /// The oldest request not taken yet.
    pub fn take(&self) -> Option<Pending> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        for pending in self.queue.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            pending.reply(Err("the session ended".to_owned()));
        }
    }
}

fn serve(mut stream: TcpStream, queue: &Mutex<VecDeque<Pending>>) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    let (tag, payload) = read_frame(&mut stream)?;
    let Some(request) = Request::decode(tag, payload) else {
        return write_frame(&mut stream, ERR, b"unknown request");
    };
    let (reply, answer) = mpsc::channel();
    queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(Pending { request, reply });
    match answer.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(data)) => write_frame(&mut stream, OK, &data),
        Ok(Err(e)) => write_frame(&mut stream, ERR, e.as_bytes()),
        Err(_) => write_frame(&mut stream, ERR, b"the host did not answer"),
    }
}

/// Sends `request` to the host at `host` and waits for its answer.
///
/// # Errors
/// If the host cannot be reached or the connection breaks; the inner
/// `Err` is the host's refusal.
pub fn request(host: SocketAddr, request: &Request) -> io::Result<Result<Vec<u8>, String>> {
    let mut stream = TcpStream::connect_timeout(&host, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(REPLY_TIMEOUT + CONNECT_TIMEOUT))?;
    write_frame(&mut stream, request.tag(), request.payload())?;
    let (tag, payload) = read_frame(&mut stream)?;
    Ok(if tag == OK { Ok(payload) } else { Err(String::from_utf8_lossy(&payload).into_owned()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_over_tcp() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let server = ControlServer::start(addr, addr.ip()).unwrap();
        let host = std::thread::spawn(move || {
            let mut answered = 0;
            while answered < 3 {
                if let Some(p) = server.take() {
                    let reply = match &p.request {
                        Request::ImportSave(data) => Ok(vec![data.len() as u8]),
                        Request::ExportSave => Ok(vec![1, 2, 3]),
                        Request::SaveState(_) | Request::LoadState(_) => {
                            Err("no states".to_owned())
                        }
                    };
                    p.reply(reply);
                    answered += 1;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        assert_eq!(request(addr, &Request::ImportSave(vec![9; 7])).unwrap(), Ok(vec![7]));
        assert_eq!(request(addr, &Request::ExportSave).unwrap(), Ok(vec![1, 2, 3]));
        assert_eq!(request(addr, &Request::LoadState(2)).unwrap(), Err("no states".to_owned()));
        host.join().unwrap();
    }
}
