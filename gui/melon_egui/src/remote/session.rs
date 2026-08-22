//! The socket, the threads that keep it fed, and the state both ends share.

use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use super::{
    MAX_DATAGRAM, Tuning,
    decoder::Decoder,
    stats::Counters,
    wire::{self, Input, Kind},
};

/// How often the host probes the link, in wall time.
const PING_INTERVAL: Duration = Duration::from_millis(500);

/// Sound waiting to be played, and the rate it arrived at.
#[derive(Default)]
pub struct AudioQueue {
    pub samples: VecDeque<i16>,
    /// The rate the host is sending at, taken from the newest datagram. The
    /// client resamples from this rather than assuming the console's rate —
    /// see [`super::audio`].
    pub rate: u32,
}

/// What both ends share.
///
/// One struct for both, because a host and a client differ only in which
/// fields they happen to use: the receive loop, the probe answering, and the
/// counters are identical. `input` is only read by a host and `audio` and
/// `decoder` only by a client, which costs an unused mutex each and saves the
/// whole thing being written twice.
pub struct Session {
    socket: UdpSocket,
    pub remote: SocketAddr,
    pub counters: Counters,
    pub shutdown: Arc<AtomicBool>,
    /// The newest controls the remote player sent — host side.
    input: Mutex<Input>,
    /// Sound waiting to be played — client side.
    audio: Mutex<AudioQueue>,
    /// The picture being rebuilt — client side.
    decoder: Mutex<Decoder>,
    /// How many sample pairs may wait before the oldest are dropped.
    audio_backlog_pairs: usize,
    /// Sequence for outgoing audio and input.
    out_seq: AtomicU32,
    /// The newest input sequence accepted, so an overtaking datagram cannot
    /// un-press a button a newer one pressed.
    input_seq: AtomicU32,
}

impl Session {
    /// Bind the threads that keep `socket` serviced.
    ///
    /// `ping` is set on the host: only one end needs to probe, and the other
    /// answers.
    pub fn start(
        socket: UdpSocket,
        remote: SocketAddr,
        tuning: Tuning,
        ping: bool,
    ) -> io::Result<Arc<Self>> {
        // Short enough that shutdown is prompt, long enough that an idle link
        // is not a spin loop.
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let session = Arc::new(Self {
            socket,
            remote,
            counters: Counters::default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            input: Mutex::new(Input::default()),
            audio: Mutex::new(AudioQueue::default()),
            decoder: Mutex::new(Decoder::new()),
            audio_backlog_pairs: tuning.audio_backlog_pairs(),
            out_seq: AtomicU32::new(1),
            input_seq: AtomicU32::new(0),
        });
        session.counters.set(&session.counters.audio_rate, u64::from(tuning.audio_rate));

        spawn("melon_egui-remote-rx", &session, Session::receive_loop)?;
        if ping {
            spawn("melon_egui-remote-ping", &session, Session::probe_loop)?;
        }
        Ok(session)
    }

    pub fn send(&self, bytes: &[u8]) -> bool {
        self.socket.send_to(bytes, self.remote).is_ok()
    }

    /// The next sequence number for an outgoing audio or input datagram.
    pub fn next_seq(&self) -> u32 {
        self.out_seq.fetch_add(1, Ordering::Relaxed)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The remote player's current controls.
    pub fn input(&self) -> Input {
        *self.input.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The newest picture, if anything has been painted since the last call.
    pub fn take_screens(&self) -> Option<[Vec<u32>; 2]> {
        self.decoder.lock().unwrap_or_else(|e| e.into_inner()).take_screens()
    }

    /// Everything waiting to be played, and the rate it is at.
    pub fn take_audio(&self) -> (Vec<i16>, u32) {
        let mut queue = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        (queue.samples.drain(..).collect(), queue.rate)
    }

    // -- the threads ---------------------------------------------------------

    fn receive_loop(&self) {
        let mut buffer = vec![0u8; MAX_DATAGRAM * 2];
        while !self.shutdown.load(Ordering::Relaxed) {
            let Ok((len, from)) = self.socket.recv_from(&mut buffer) else {
                continue;
            };
            if from != self.remote {
                continue;
            }
            self.receive(&buffer[..len]);
        }
    }

    fn probe_loop(&self) {
        while !self.shutdown.load(Ordering::Relaxed) {
            self.send(&wire::encode_ping(wire::wall_clock_micros()));
            std::thread::sleep(PING_INTERVAL);
        }
    }

    fn receive(&self, datagram: &[u8]) {
        let counters = &self.counters;
        let Some(kind) = wire::kind_of(datagram) else {
            counters.bump(&counters.discarded, 1);
            return;
        };
        match kind {
            Kind::Video => self.receive_video(datagram),
            Kind::Audio => self.receive_audio(datagram),
            Kind::Input => self.receive_input(datagram),
            // Answered here rather than queued: the probe is timing the path,
            // and time spent waiting for another thread to notice it is not
            // part of the path.
            Kind::Ping => {
                self.send(&wire::echo_ping(datagram));
            }
            Kind::Pong => {
                if let Some(sent) = wire::read_pong(datagram)
                    && let Some(rtt) = wire::wall_clock_micros().checked_sub(sent)
                {
                    counters.observe_rtt(Duration::from_micros(rtt));
                }
            }
            Kind::Hello | Kind::Welcome => {}
        }
    }

    fn receive_video(&self, datagram: &[u8]) {
        let counters = &self.counters;
        let mut decoder = self.decoder.lock().unwrap_or_else(|e| e.into_inner());
        let before = decoder.frames_seen;
        if !decoder.apply(datagram) {
            counters.bump(&counters.discarded, 1);
            return;
        }
        counters.bump(&counters.video_datagrams, 1);
        counters.bump(&counters.video_bytes, datagram.len() as u64);
        if decoder.frames_seen != before {
            counters.bump(&counters.frames, 1);
        }
    }

    fn receive_audio(&self, datagram: &[u8]) {
        let counters = &self.counters;
        let Some((rate, samples)) = wire::decode_audio(datagram) else {
            counters.bump(&counters.discarded, 1);
            return;
        };
        let mut queue = self.audio.lock().unwrap_or_else(|e| e.into_inner());
        queue.rate = rate;
        queue.samples.extend(samples.iter().copied());
        // Audio that is queued is audio that is late. Trimming from the front
        // costs one audible moment; not trimming costs a sound track that
        // slides further behind the picture for as long as the session runs.
        let limit = self.audio_backlog_pairs * 2;
        if queue.samples.len() > limit {
            let excess = queue.samples.len() - limit;
            queue.samples.drain(..excess);
            counters.bump(&counters.audio_dropped, excess as u64 / 2);
        }
        counters.bump(&counters.audio_pairs, samples.len() as u64 / 2);
        counters.set(&counters.audio_rate, u64::from(rate));
    }

    fn receive_input(&self, datagram: &[u8]) {
        let counters = &self.counters;
        let Some(input) = wire::decode_input(datagram) else {
            counters.bump(&counters.discarded, 1);
            return;
        };
        let newest = self.input_seq.load(Ordering::Relaxed);
        if newest != 0 && input.seq.wrapping_sub(newest) > u32::MAX / 2 {
            counters.bump(&counters.discarded, 1);
            return;
        }
        self.input_seq.store(input.seq, Ordering::Relaxed);
        *self.input.lock().unwrap_or_else(|e| e.into_inner()) = input;
        counters.bump(&counters.inputs, 1);
    }
}

/// Run `body` on a named thread for as long as the session lives.
fn spawn(name: &str, session: &Arc<Session>, body: fn(&Session)) -> io::Result<()> {
    let session = Arc::clone(session);
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || body(&session))
        .map(|_| ())
        .map_err(|error| io::Error::other(format!("cannot start {name}: {error}")))
}

/// Answer a client's `HELLO` on `socket`, returning where it came from.
///
/// Blocks until one arrives, so the caller runs it off the UI thread.
pub fn accept_hello(socket: &UdpSocket) -> io::Result<SocketAddr> {
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut buffer = vec![0u8; MAX_DATAGRAM];
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((len, client)) if wire::kind_of(&buffer[..len]) == Some(Kind::Hello) => {
                let welcome = wire::header(Kind::Welcome);
                // Three copies: losing the welcome costs the client its whole
                // connection attempt, and it is five bytes.
                for _ in 0..3 {
                    socket.send_to(&welcome, client)?;
                }
                return Ok(client);
            }
            Ok(_) => continue,
            Err(ref error) if would_block(error) => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Announce to `host` on `socket` until it answers.
///
/// Retries, because on a VPN the first datagram after the tunnel comes up is
/// the one most likely to be dropped.
pub fn exchange_hello(socket: &UdpSocket, host: SocketAddr) -> io::Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    let hello = wire::header(Kind::Hello);
    let mut buffer = vec![0u8; MAX_DATAGRAM];
    for _ in 0..10 {
        socket.send_to(&hello, host)?;
        match socket.recv_from(&mut buffer) {
            Ok((len, from))
                if from == host && wire::kind_of(&buffer[..len]) == Some(Kind::Welcome) =>
            {
                return Ok(());
            }
            Ok(_) => continue,
            Err(ref error) if would_block(error) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, format!("no answer from {host} after 10 attempts")))
}

/// Whether a socket error is just the read timeout expiring.
fn would_block(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}
