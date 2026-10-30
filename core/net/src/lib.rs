//! Multiplayer transport for the emulated DS Wi-Fi.
//!
//! ```text
//!  ds_peripherals::wifi::WiFi  (TX slots, RX buffer, MP CMD/reply/ACK)
//!    |  dyn MpLink
//!    v
//!  NetLink (this crate)        stall budget, per-frame mp::process()
//!    |  melonds_net::mp::MpHandle      (C ABI)
//!    v
//!  melonDS MPInterface:  Local  consoles in this process
//!                        Lan    other processes / machines over ENet
//!                               (UDP 7064, discovery on UDP 7063)
//! ```
//!
//! Select the backend with [`init`] before any console powers its Wi-Fi on,
//! then give each emulator a [`NetLink`] (`Emulator::set_mp_link`). LAN
//! sessions are opened and closed through [`lan`].

mod link;

pub use link::NetLink;
pub use melonds_net::{
    lan,
    mp::{MAX_INSTANCES, MpBackend},
};

/// Routes melonDS's network log to the `log` crate and selects the
/// process-wide MP backend.
///
/// Call it before consoles start (or after they stop): the backend must not
/// change while a console is on the medium.
pub fn init(backend: MpBackend) {
    melonds_net::init_logging();
    melonds_net::mp::set_backend(backend);
}

/// The current MP backend.
pub fn backend() -> MpBackend {
    melonds_net::mp::backend()
}

/// Services the backend (LAN: ENet connections, discovery, pings). Called
/// by [`NetLink`] once per emulated frame; call it yourself while no console
/// is running but a LAN session is open, or peers time out.
pub fn process() {
    melonds_net::mp::process();
}
