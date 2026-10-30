//! Multiplayer pieces for the GUIs, on top of `ds_net`.
//!
//! ```text
//!  GUI (egui)
//!    |-- LanSession   host / join (worker thread) / leave a melonDS LAN
//!    |                session; players and their ENet ping
//!    |-- ReadyProbe   UDP ping/pong (port 7065..=7068) once a second:
//!    |                is the peer there, and is it ready to play?
//!    v                -> green / red light
//!  ds_net  (NetLink per console, melonds_net LAN backend)
//! ```
//!
//! Ports to open in a firewall: UDP 7063 (LAN discovery), UDP 7064 (ENet,
//! MP frames) and UDP 7065..=7068 (readiness probe).

mod light;
mod ready;
mod session;

pub use ds_net::lan::{Player, PlayerStatus};
pub use light::{Light, PeerLight, ReadyLight, host_part};
pub use ready::{PROBE_PORT, PROBE_PORTS, PeerState, Readiness, ReadyProbe, Want};
pub use session::{LanSession, MAX_PLAYERS, SessionState};

/// This machine's LAN address (the one other machines join), if any. No
/// packet is sent: connecting a UDP socket only picks the outgoing route.
pub fn local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((std::net::Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}
