//! Internet play: emulated Ethernet frames between the DS's
//! Wi-Fi adapter and a real network.
//!
//! Corresponds to melonDS's `src/net/Net.{h,cpp}`, `NetDriver.h` and
//! `PacketDispatcher.{h,cpp}`.
//!
//! # Not ported
//! melonDS's two concrete drivers are deliberately absent:
//!
//! * `Net_PCap.cpp` — dynamically loads libpcap and bridges onto a
//!   physical adapter. It is almost entirely FFI plumbing plus adapter
//!   enumeration.
//! * `Net_Slirp.cpp` — a complete user-mode TCP/IP stack supplied by
//!   libslirp, plus DNS frame rewriting.
//!
//! Both would pull a C library into `nds-core`. The [`NetDriver`] trait is
//! the seam where a frontend can add either one; [`NullNetDriver`] and
//! [`LoopbackNetDriver`] cover the "no internet" and "test" cases.
//!
//! Note that this module is about the *internet* path. The DS Wi-Fi
//! **hardware** registers live in `crate::hw::wifi`, a separate and
//! unrelated module.

mod net;
mod net_driver;
mod packet_dispatcher;

pub use net::Net;
pub use net_driver::{LoopbackNetDriver, NetDriver, NullNetDriver, RxCallback};
pub use packet_dispatcher::{
    DispatchedPacket, EXTERNAL_SENDER, PACKET_QUEUE_SIZE, PacketDispatcher,
};
