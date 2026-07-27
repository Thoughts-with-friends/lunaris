//! LAN room hosting/joining and Wi-Fi MP frame relay for `lunaris`.
//!
//! This crate is the only place in the workspace that touches sockets for
//! multiplayer; `nds-core` stays network-free and only depends on the
//! [`nds_core::nds::MpTransport`] trait boundary. See
//! `docs/design/design_lan.md` for the full design.
//!
//! Entry points: [`room::Room::host`] / [`room::Room::join`].

pub mod pacing;
pub mod room;
pub mod transport;
pub mod wire;

pub use room::{PlayerView, Room, RoomConfig, RoomHandle};
pub use transport::NetTransport;
pub use wire::LinkParams;
