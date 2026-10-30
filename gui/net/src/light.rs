//! The multiplayer "ready" light: what to ping and how to read the answers.
//!
//! ```text
//!  session state          targets pinged                     green when
//!  ---------------------  ---------------------------------  -----------------------------
//!  offline / failed       the host address typed by the user  it answers, hosts a session
//!                                                             with room, same game (Want::
//!                                                             Joinable): "Join" will work
//!  hosting                every client in the session         all answer and are in the
//!  joined                 every other player (host included)  session, same game (Want::
//!                                                             InSession): ready to play
//!  joining                (unchanged)                         -
//! ```
//!
//! Anything else is red, with the reason.

use std::{
    net::{IpAddr, ToSocketAddrs as _},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use crate::{LanSession, MAX_PLAYERS, PeerState, Readiness, ReadyProbe, SessionState, Want};

/// One peer's line in the GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLight {
    pub label: String,
    pub address: IpAddr,
    pub readiness: Readiness,
}

/// The overall verdict plus the per-peer ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Light {
    /// Green.
    pub ready: bool,
    /// Short explanation ("ready (12 ms)", "no answer", ...).
    pub text: String,
    pub peers: Vec<PeerLight>,
}

/// How long the address box must stay unchanged before a host name is
/// looked up (no lookup per keystroke).
const RESOLVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// Evaluates the light each GUI frame. The typed address is resolved without
/// ever blocking the caller: IP literals are parsed on the spot, host names
/// are looked up on a worker thread once typing pauses.
#[derive(Debug, Default)]
pub struct ReadyLight {
    typed: String,
    /// When `typed` last changed.
    typed_at: Option<Instant>,
    resolved: Option<IpAddr>,
    /// DNS lookup in flight for the current `typed`.
    lookup: Option<Receiver<Option<IpAddr>>>,
    /// A lookup is still to be started for the current `typed`.
    lookup_pending: bool,
}

impl ReadyLight {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tells `probe` what this console is and whom to ping, and returns the
    /// light. `join_address` is the host address box (`host[:port]`);
    /// `local` carries `running` and `game_code` (the session fields are
    /// filled in here).
    pub fn evaluate(
        &mut self,
        session: &LanSession,
        probe: &ReadyProbe,
        join_address: &str,
        mut local: PeerState,
    ) -> Light {
        let state = session.state();
        local.hosting = *state == SessionState::Hosting;
        local.joined = matches!(state, SessionState::Joined(_));
        local.players = session.players().len().min(255) as u8;
        local.max_players = if local.hosting { MAX_PLAYERS } else { 0 };
        probe.set_local(local);

        let (want, targets): (Want, Vec<(String, IpAddr)>) = if state.in_session() {
            let others = session
                .players()
                .iter()
                .filter(|p| !p.is_local && !p.address.is_unspecified())
                .map(|p| (p.name.clone(), IpAddr::V4(p.address)))
                .collect();
            (Want::InSession, others)
        } else {
            let target = self.resolve(join_address);
            (
                Want::Joinable,
                target.map(|ip| (join_address.trim().to_owned(), ip)).into_iter().collect(),
            )
        };
        let ips: Vec<IpAddr> = targets.iter().map(|(_, ip)| *ip).collect();
        probe.set_targets(&ips);

        if matches!(state, SessionState::Joining(_)) {
            return Light { ready: false, text: "joining...".to_owned(), peers: Vec::new() };
        }
        if targets.is_empty() {
            let text = match (state, join_address.trim().is_empty()) {
                (SessionState::Hosting, _) => "waiting for players",
                (SessionState::Joined(_), _) => "nobody else in the session",
                (_, true) => "enter the host's address",
                _ if self.resolving() => "resolving the address...",
                (_, false) => "cannot resolve the address",
            };
            return Light { ready: false, text: text.to_owned(), peers: Vec::new() };
        }

        let peers: Vec<PeerLight> = targets
            .into_iter()
            .map(|(label, ip)| PeerLight {
                label,
                address: ip,
                readiness: probe.readiness(ip, want, local.game_code),
            })
            .collect();
        let ready = peers.iter().all(|p| p.readiness.is_ready());
        let text = if ready {
            let worst = peers
                .iter()
                .filter_map(|p| match p.readiness {
                    Readiness::Ready { rtt, .. } => Some(rtt),
                    Readiness::NotReady(_) => None,
                })
                .max()
                .unwrap_or(Duration::ZERO);
            let what =
                if want == Want::Joinable { "host ready, you can join" } else { "ready to play" };
            format!("{what} ({} ms)", worst.as_millis())
        } else {
            peers
                .iter()
                .find_map(|p| match &p.readiness {
                    Readiness::NotReady(why) => Some(format!("{}: {why}", p.label)),
                    Readiness::Ready { .. } => None,
                })
                .unwrap_or_default()
        };
        Light { ready, text, peers }
    }

    /// The host part of `host[:port]` as an address, if known yet. Never
    /// blocks: see [`ReadyLight`].
    fn resolve(&mut self, typed: &str) -> Option<IpAddr> {
        let typed = typed.trim();
        if typed != self.typed {
            self.typed = typed.to_owned();
            self.typed_at = Some(Instant::now());
            // A lookup for the old text is abandoned (its thread finishes
            // on its own; the result is dropped with the receiver).
            self.lookup = None;
            let host = host_part(typed);
            self.resolved = host.parse::<IpAddr>().ok();
            // Digits and dots only: an IP being typed, not a host name.
            let partial_ip = host.chars().all(|c| c.is_ascii_digit() || c == '.');
            self.lookup_pending = self.resolved.is_none() && !host.is_empty() && !partial_ip;
        }

        if self.lookup_pending && self.typed_at.is_some_and(|at| at.elapsed() >= RESOLVE_DEBOUNCE) {
            self.lookup_pending = false;
            let (tx, rx) = mpsc::channel();
            let typed = self.typed.clone();
            std::thread::spawn(move || {
                let _ = tx.send(resolve_host(&typed));
            });
            self.lookup = Some(rx);
        }

        if let Some(rx) = &self.lookup {
            match rx.try_recv() {
                Ok(ip) => {
                    self.resolved = ip;
                    self.lookup = None;
                }
                Err(TryRecvError::Disconnected) => self.lookup = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        self.resolved
    }

    /// A host name lookup is pending or in flight.
    pub const fn resolving(&self) -> bool {
        self.lookup_pending || self.lookup.is_some()
    }
}

/// The host part of a typed address: `"192.168.0.2:7064"` -> `"192.168.0.2"`,
/// `"[::1]:7064"` -> `"::1"`, `"pc-name"` -> `"pc-name"`. The LAN port is
/// fixed (7064), so a port suffix is dropped.
pub fn host_part(text: &str) -> String {
    let text = text.trim();
    if let Ok(addr) = text.parse::<std::net::SocketAddr>() {
        return addr.ip().to_string();
    }
    match text.split_once(':') {
        Some((host, port)) if !port.contains(':') => host.to_owned(),
        _ => text.to_owned(),
    }
}

/// `"192.168.0.2"`, `"pc-name"`, `"[::1]:7064"`, `"host:7064"` -> address.
fn resolve_host(text: &str) -> Option<IpAddr> {
    let host = host_part(text);
    if host.is_empty() {
        return None;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    (host.as_str(), 0).to_socket_addrs().ok()?.find(std::net::SocketAddr::is_ipv4).map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_typed_addresses() {
        let local = Some(IpAddr::from([127, 0, 0, 1]));
        assert_eq!(resolve_host("127.0.0.1"), local);
        assert_eq!(resolve_host(" 127.0.0.1:7064 ".trim()), local);
        assert_eq!(resolve_host("localhost"), local);
        assert_eq!(resolve_host(""), None);
        assert_eq!(host_part("10.0.0.2:7064"), "10.0.0.2");
        assert_eq!(host_part("[::1]:7064"), "::1");
        assert_eq!(host_part("::1"), "::1");
        assert_eq!(host_part(" pc-name "), "pc-name");
    }

    #[test]
    fn typing_never_blocks() {
        let mut light = ReadyLight::new();
        // Literal IPs resolve at once, partial ones are never looked up.
        assert_eq!(light.resolve("127.0.0.1"), Some(IpAddr::from([127, 0, 0, 1])));
        assert_eq!(light.resolve("192.168.0."), None);
        assert!(!light.resolving());
        // A host name is looked up in the background after the debounce.
        let start = Instant::now();
        assert_eq!(light.resolve("localhost"), None);
        assert!(light.resolving());
        assert!(start.elapsed() < Duration::from_millis(50), "must not block");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = None;
        while got.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            got = light.resolve("localhost");
        }
        assert!(got.is_some());
    }
}
