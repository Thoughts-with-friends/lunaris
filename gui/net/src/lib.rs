//! LAN room hosting/joining and Wi-Fi MP frame relay for `lunaris`.
//!
//! This crate is the only place in the workspace that touches sockets for
//! multiplayer; `nds-core` stays network-free and only depends on the
//! [`nds_core::nds::MpTransport`] trait boundary. See
//! `docs/design/design_lan.md` for the full design.
//!
//! Entry points: [`room::Room::host`] / [`room::Room::join`] to build a
//! LAN room, then [`mp_interface::MpInterfaceSelector`] to install it (or
//! any other MP backend from [`nds_core::net`]) on a running emulator.

pub mod lan;
pub mod mp_interface;
pub mod pacing;
pub mod room;
pub mod transport;
pub mod wire;

pub use lan::{DiscoveryData, Lan, LanSession, Player, PlayerStatus};
pub use mp_interface::MpInterfaceSelector;
pub use room::{PlayerView, Room, RoomConfig, RoomHandle};
pub use transport::NetTransport;
pub use wire::LinkParams;
