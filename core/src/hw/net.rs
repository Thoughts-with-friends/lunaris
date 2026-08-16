//! DS networking: the two independent paths a game can use to talk to
//! something other than itself.
//!
//! Structurally a port of melonDS's `src/net/` directory (vendored for
//! reference at `docs/design/melonds/net/`), split the way the DS itself
//! splits the feature:
//!
//! | Module | melonDS source | Purpose |
//! |---|---|---|
//! | [`mp_interface`] | `MPInterface.{h,cpp}` | Backend-agnostic MP frame transport |
//! | [`local`] | `LocalMP.{h,cpp}` | Local wireless play between instances in one process |
//! | [`wifi`] | `Net.{h,cpp}`, `NetDriver.h`, `PacketDispatcher.{h,cpp}` | Internet play (Nintendo WFC) |
//! | [`bridge`] | *(lunaris-only)* | Adapts an [`mp_interface::MpInterface`] to the hardware's `MpTransport` |
//!
//! # What lives elsewhere, and why
//! `nds-core` owns no sockets — a constraint the Wi-Fi hardware module and
//! the `lunaris_net` crate both document and depend on. So the two melonDS
//! files that *are* socket code stay in the frontend:
//!
//! * `LAN.cpp` (enet-based room hosting, discovery, player list) is
//!   `lunaris_net`'s `room`/`transport`/`wire` modules.
//! * `Netplay.cpp` (savestate-synchronised netplay) has no lunaris
//!   equivalent and is not ported.
//!
//! Likewise `Net_PCap.cpp` and `Net_Slirp.cpp` are C-library bindings; see
//! [`wifi`] for what stands in for them.
//!
//! # Naming
//! [`wifi`] here means *internet play*, matching melonDS's `Net.cpp`. The
//! DS Wi-Fi **hardware registers** are a different module,
//! `crate::hw::wifi`.

pub mod bridge;
pub mod local;
pub mod mp_interface;
pub mod wifi;

pub use bridge::MpInterfaceTransport;
pub use local::{LocalMp, LocalMpHub};
pub use mp_interface::{
    DummyMp, MAX_INSTANCES, MpFrameCategory, MpFrameType, MpInterface, MpInterfaceType,
    MpPacketHeader, MpRecvResult,
};
pub use wifi::{Net, NetDriver, NullNetDriver, PacketDispatcher, set_assoc_trace};
