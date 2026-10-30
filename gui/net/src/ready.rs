//! Readiness probe: is the other console there, and ready to play?
//!
//! ```text
//!  responder thread                      pinger thread (every PING_INTERVAL)
//!  UDP PROBE_PORT.. (first free)         UDP ephemeral
//!     <-------- PING (node, seq) -------- to every target, ports
//!     --------- PONG (node, seq, ------->   PROBE_PORT..PROBE_PORT+PROBE_PORTS
//!               hosting/joined/running,     (several instances per PC)
//!               players, game code)      round trip -> peers[addr]
//! ```
//!
//! [`ReadyProbe::readiness`] turns the latest pong of a target into a
//! verdict: the peer answered within [`STALE_AFTER`] and is in the state the
//! caller needs ([`Want`]), with the same game loaded if both know theirs.
//! The GUI shows it as a green (ready) or red (not ready) light.
//!
//! Wire format (little-endian): [`MAGIC`], version, kind (0 ping, 1 pong),
//! node id (u64, random per process) and sequence (u32); a pong adds flags
//! (bit 0 hosting, 1 joined, 2 running), players, max players and the 4-byte
//! game code.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// First UDP port a responder tries.
pub const PROBE_PORT: u16 = 7065;
/// Ports tried after it (one per instance on the same machine).
pub const PROBE_PORTS: u16 = 4;

const MAGIC: [u8; 4] = *b"CGRY";
const VERSION: u8 = 1;
const PING_LEN: usize = 4 + 1 + 1 + 8 + 4;
const PONG_LEN: usize = PING_LEN + 1 + 1 + 1 + 4;

/// How often targets are pinged.
const PING_INTERVAL: Duration = Duration::from_secs(1);
/// A peer that has not answered for this long is not ready.
pub const STALE_AFTER: Duration = Duration::from_millis(3500);
/// Socket read timeout, i.e. how fast the threads notice `stop`.
const POLL: Duration = Duration::from_millis(100);

/// What a console reports about itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeerState {
    pub hosting: bool,
    pub joined: bool,
    /// A game is running.
    pub running: bool,
    pub players: u8,
    pub max_players: u8,
    /// Game code of the running/selected cartridge (header 0x0C), zero if
    /// none.
    pub game_code: [u8; 4],
}

impl PeerState {
    const fn flags(&self) -> u8 {
        (self.hosting as u8) | ((self.joined as u8) << 1) | ((self.running as u8) << 2)
    }

    /// The game code as text, if any.
    pub fn game(&self) -> Option<String> {
        (self.game_code != [0; 4]).then(|| String::from_utf8_lossy(&self.game_code).into_owned())
    }
}

/// What the caller needs the peer to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Hosting a session with room for one more (before joining it).
    Joinable,
    /// In our session (hosting it or joined to it).
    InSession,
}

/// The verdict for one peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Green: answering, in the wanted state.
    Ready { rtt: Duration, peer: PeerState },
    /// Red, with the reason.
    NotReady(String),
}

impl Readiness {
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    rtt: Duration,
    at: Instant,
    state: PeerState,
    /// The answering process (one process may answer from several
    /// addresses: loopback and its LAN address).
    node: u64,
}

#[derive(Debug, Default)]
struct Shared {
    local: Mutex<PeerState>,
    targets: Mutex<Vec<IpAddr>>,
    /// Also ping the LAN broadcast address and loopback, to find hosts.
    discover: AtomicBool,
    /// Latest pong per responder.
    peers: Mutex<HashMap<SocketAddr, Seen>>,
    /// Called after each pong (e.g. to repaint the GUI).
    notify: Mutex<Option<Notify>>,
}

struct Notify(Box<dyn Fn() + Send>);

impl std::fmt::Debug for Notify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notify")
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The probe: a responder and a pinger thread, stopped on drop.
#[derive(Debug)]
pub struct ReadyProbe {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    responder_port: Option<u16>,
}

impl ReadyProbe {
    /// Starts the threads. Without a free port in
    /// `PROBE_PORT..PROBE_PORT + PROBE_PORTS` this console cannot be probed
    /// (but can still probe others); see [`Self::responder_port`].
    pub fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        let node = node_id();
        let mut threads = Vec::new();

        let responder = (PROBE_PORT..PROBE_PORT + PROBE_PORTS)
            .find_map(|port| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).ok());
        let responder_port = responder.as_ref().and_then(|s| s.local_addr().ok()).map(|a| a.port());
        match (responder, responder_port) {
            (Some(socket), Some(port)) => {
                log::info!("ready probe: answering on UDP {port}");
                let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
                threads.push(std::thread::spawn(move || {
                    respond(&socket, &shared, &stop, node);
                }));
            }
            _ => log::warn!(
                "ready probe: UDP {PROBE_PORT}..{} all taken; others cannot probe this console",
                PROBE_PORT + PROBE_PORTS - 1
            ),
        }

        match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
            Ok(socket) => {
                let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
                threads.push(std::thread::spawn(move || {
                    ping(&socket, &shared, &stop, node);
                }));
            }
            Err(e) => log::error!("ready probe: cannot open a UDP socket: {e}"),
        }

        Self { shared, stop, threads, responder_port }
    }

    /// The port this console answers on, if any.
    pub const fn responder_port(&self) -> Option<u16> {
        self.responder_port
    }

    /// What this console reports to others.
    pub fn set_local(&self, state: PeerState) {
        *lock(&self.shared.local) = state;
    }

    /// The machines to ping (replaces the previous list).
    pub fn set_targets(&self, targets: &[IpAddr]) {
        let mut current = lock(&self.shared.targets);
        if current.as_slice() != targets {
            *current = targets.to_vec();
        }
    }

    /// Whether to look for hosts: pings also go to the LAN broadcast
    /// address and to this machine, see [`Self::hosts`].
    pub fn set_discover(&self, on: bool) {
        self.shared.discover.store(on, Ordering::Relaxed);
    }

    /// Hosts that answered recently (one address per host process,
    /// preferring its LAN address over loopback), nearest first.
    pub fn hosts(&self) -> Vec<(IpAddr, PeerState)> {
        let peers = lock(&self.shared.peers);
        let mut best: HashMap<u64, (SocketAddr, Seen)> = HashMap::new();
        for (addr, seen) in peers.iter() {
            if !seen.state.hosting || seen.at.elapsed() >= STALE_AFTER {
                continue;
            }
            let better = best
                .get(&seen.node)
                .is_none_or(|(old, _)| old.ip().is_loopback() && !addr.ip().is_loopback());
            if better {
                best.insert(seen.node, (*addr, *seen));
            }
        }
        drop(peers);
        let mut hosts: Vec<(IpAddr, Seen)> = best.into_values().map(|(a, s)| (a.ip(), s)).collect();
        hosts.sort_by_key(|(ip, seen)| (ip.is_loopback(), seen.rtt));
        hosts.into_iter().map(|(ip, seen)| (ip, seen.state)).collect()
    }

    /// Called from the pinger thread after every pong.
    pub fn on_update(&self, notify: impl Fn() + Send + 'static) {
        *lock(&self.shared.notify) = Some(Notify(Box::new(notify)));
    }

    /// The verdict for `target`, comparing game codes with `local_game`
    /// (zero: unknown).
    pub fn readiness(&self, target: IpAddr, want: Want, local_game: [u8; 4]) -> Readiness {
        let peers = lock(&self.shared.peers);
        let fresh: Vec<Seen> = peers
            .iter()
            .filter(|(addr, seen)| addr.ip() == target && seen.at.elapsed() < STALE_AFTER)
            .map(|(_, seen)| *seen)
            .collect();
        drop(peers);
        if fresh.is_empty() {
            return Readiness::NotReady("no answer".to_owned());
        }

        let mut reason = String::new();
        for Seen { rtt, state, .. } in fresh {
            let wanted = match want {
                Want::Joinable if !state.hosting => Err("not hosting"),
                Want::Joinable if state.players >= state.max_players => Err("session full"),
                Want::InSession if !state.hosting && !state.joined => Err("not in the session"),
                _ => Ok(()),
            };
            let same_game =
                local_game == [0; 4] || state.game_code == [0; 4] || state.game_code == local_game;
            match (wanted, same_game) {
                (Ok(()), true) => return Readiness::Ready { rtt, peer: state },
                (Ok(()), false) => {
                    reason = format!("different game ({})", state.game().unwrap_or_default());
                }
                (Err(why), _) if reason.is_empty() => reason = why.to_owned(),
                (Err(_), _) => {}
            }
        }
        Readiness::NotReady(reason)
    }
}

impl Drop for ReadyProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// A random-enough id telling this process's pongs apart from others'.
fn node_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish()
}

fn header(kind: u8, node: u64, seq: u32) -> Vec<u8> {
    let mut msg = Vec::with_capacity(PONG_LEN);
    msg.extend_from_slice(&MAGIC);
    msg.push(VERSION);
    msg.push(kind);
    msg.extend_from_slice(&node.to_le_bytes());
    msg.extend_from_slice(&seq.to_le_bytes());
    msg
}

/// `(kind, node, seq)` of a probe datagram.
fn parse(msg: &[u8]) -> Option<(u8, u64, u32)> {
    if msg.len() < PING_LEN || msg[..4] != MAGIC || msg[4] != VERSION {
        return None;
    }
    let node = u64::from_le_bytes(msg[6..14].try_into().ok()?);
    let seq = u32::from_le_bytes(msg[14..18].try_into().ok()?);
    Some((msg[5], node, seq))
}

/// Responder thread: answer pings with this console's state.
fn respond(socket: &UdpSocket, shared: &Shared, stop: &AtomicBool, node: u64) {
    let _ = socket.set_read_timeout(Some(POLL));
    let mut buf = [0_u8; 64];
    while !stop.load(Ordering::Relaxed) {
        let Ok((len, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let Some((0, _, seq)) = parse(&buf[..len]) else {
            continue;
        };
        let local = *lock(&shared.local);
        let mut pong = header(1, node, seq);
        pong.extend_from_slice(&[local.flags(), local.players, local.max_players]);
        pong.extend_from_slice(&local.game_code);
        let _ = socket.send_to(&pong, from);
    }
}

/// Pinger thread: ping every target once per interval, collect pongs.
fn ping(socket: &UdpSocket, shared: &Shared, stop: &AtomicBool, node: u64) {
    let _ = socket.set_read_timeout(Some(POLL));
    let _ = socket.set_broadcast(true);
    let mut buf = [0_u8; 64];
    let mut seq: u32 = 0;
    // Send time of the recent sequence numbers.
    let mut sent: HashMap<u32, Instant> = HashMap::new();
    let mut next = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        if Instant::now() >= next {
            next += PING_INTERVAL;
            seq = seq.wrapping_add(1);
            sent.insert(seq, Instant::now());
            sent.retain(|_, at| at.elapsed() < STALE_AFTER);
            let msg = header(0, node, seq);
            let mut targets = lock(&shared.targets).clone();
            if shared.discover.load(Ordering::Relaxed) {
                targets.push(IpAddr::V4(Ipv4Addr::BROADCAST));
                targets.push(IpAddr::V4(Ipv4Addr::LOCALHOST));
            }
            for ip in targets {
                for port in PROBE_PORT..PROBE_PORT + PROBE_PORTS {
                    let _ = socket.send_to(&msg, SocketAddr::new(ip, port));
                }
            }
        }

        let Ok((len, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let msg = &buf[..len];
        let Some((1, peer, seq)) = parse(msg) else {
            continue;
        };
        if peer == node || msg.len() < PONG_LEN {
            continue; // our own responder (same machine)
        }
        let Some(rtt) = sent.get(&seq).map(Instant::elapsed) else {
            continue;
        };
        let flags = msg[PING_LEN];
        let state = PeerState {
            hosting: flags & 1 != 0,
            joined: flags & 2 != 0,
            running: flags & 4 != 0,
            players: msg[PING_LEN + 1],
            max_players: msg[PING_LEN + 2],
            game_code: [msg[PING_LEN + 3], msg[PING_LEN + 4], msg[PING_LEN + 5], msg[PING_LEN + 6]],
        };
        lock(&shared.peers).insert(from, Seen { rtt, at: Instant::now(), state, node: peer });
        if let Some(Notify(notify)) = lock(&shared.notify).as_ref() {
            notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two probes on this machine see each other over loopback.
    #[test]
    fn probes_see_each_other() {
        let host = ReadyProbe::start();
        let guest = ReadyProbe::start();
        if host.responder_port().is_none() || guest.responder_port().is_none() {
            return; // ports taken by running emulators; nothing to test
        }
        host.set_local(PeerState {
            hosting: true,
            players: 1,
            max_players: 16,
            game_code: *b"ABCE",
            ..PeerState::default()
        });
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        guest.set_targets(&[loopback]);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut verdict = guest.readiness(loopback, Want::Joinable, *b"ABCE");
        while !verdict.is_ready() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            verdict = guest.readiness(loopback, Want::Joinable, *b"ABCE");
        }
        assert!(verdict.is_ready(), "{verdict:?}");

        // Discovery finds the host without being told its address.
        let finder = ReadyProbe::start();
        finder.set_discover(true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while finder.hosts().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(finder.hosts().len(), 1, "one host process: {:?}", finder.hosts());
        drop(finder);

        // Same peer, different game: red.
        let other = guest.readiness(loopback, Want::Joinable, *b"WXYZ");
        assert!(!other.is_ready());

        // Host stops hosting: red once the next pong says so.
        host.set_local(PeerState::default());
        let deadline = Instant::now() + Duration::from_secs(5);
        while guest.readiness(loopback, Want::Joinable, [0; 4]).is_ready()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            guest.readiness(loopback, Want::Joinable, [0; 4]),
            Readiness::NotReady("not hosting".to_owned())
        );
    }
}
