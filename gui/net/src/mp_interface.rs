//! Frontend-side selection of the active MP backend.
//!
//! Port of melonDS's `MPInterface::Set` / `MPInterface::Get`
//! (`docs/design/melonds/net/MPInterface.cpp`), minus the process-global
//! `unique_ptr`: the selected backend is owned by an
//! [`MpInterfaceSelector`] value that the UI holds, and installed on the
//! emulator through the existing [`NDS::set_mp_transport`] API.
//!
//! The three selectable backends map onto melonDS's enum as follows:
//!
//! | [`MpInterfaceType`] | melonDS | lunaris implementation |
//! |---|---|---|
//! | `Dummy` | `DummyMP` | no transport installed |
//! | `Local` | `LocalMP` | [`nds_core::net::LocalMp`] over a shared hub |
//! | `Lan` | `LAN` (enet) | [`crate::Room`] + [`crate::NetTransport`] |
//!
//! `Lan` is the only backend `nds-core` cannot provide itself, because it
//! is the only one that owns sockets — see [`nds_core::net`].

use std::sync::Arc;

use nds_core::{
    NDS,
    nds::MpTransport,
    net::{LocalMp, LocalMpHub, MpInterfaceTransport, MpInterfaceType},
};

use crate::room::{Room, RoomHandle};

/// Owns whichever MP backend is currently installed on an [`NDS`], and the
/// state that outlives a single transport (the local hub, the room
/// handle).
///
/// The transport half of each backend is handed to
/// [`NDS::set_mp_transport`] and not retained here, matching how
/// `lan_room.rs` already treats [`Room`].
#[derive(Default)]
pub struct MpInterfaceSelector {
    kind: MpInterfaceType,
    /// Retained so a second instance can join the same local session with
    /// [`MpInterfaceSelector::set_local`].
    local_hub: Option<Arc<LocalMpHub>>,
    /// Retained so the UI can poll the player list and link stats.
    room: Option<RoomHandle>,
}

impl MpInterfaceSelector {
    /// Creates a selector with no backend installed
    /// ([`MpInterfaceType::Dummy`]).
    #[must_use]
    pub const fn new() -> Self {
        MpInterfaceSelector { kind: MpInterfaceType::Dummy, local_hub: None, room: None }
    }

    /// Which backend is currently installed.
    #[must_use]
    pub const fn kind(&self) -> MpInterfaceType {
        self.kind
    }

    /// A short label for the current backend, for UI display.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self.kind {
            MpInterfaceType::Dummy => "off",
            MpInterfaceType::Local => "local",
            MpInterfaceType::Lan => "LAN",
            MpInterfaceType::Netplay => "netplay",
        }
    }

    /// The live room handle, when [`MpInterfaceType::Lan`] is selected.
    #[must_use]
    pub const fn room(&self) -> Option<&RoomHandle> {
        self.room.as_ref()
    }

    /// The local hub, when [`MpInterfaceType::Local`] is selected. Pass it
    /// to another selector's [`MpInterfaceSelector::set_local`] to put a
    /// second instance in the same session.
    #[must_use]
    pub const fn local_hub(&self) -> Option<&Arc<LocalMpHub>> {
        self.local_hub.as_ref()
    }

    /// Removes any installed backend.
    ///
    /// melonDS installs a no-op `DummyMP` object here; lunaris's Wi-Fi
    /// hardware already treats "no transport" as the same thing, so this
    /// clears the transport instead of installing a do-nothing one.
    pub fn set_dummy(&mut self, nds: &mut NDS) {
        nds.set_mp_transport(None);
        self.kind = MpInterfaceType::Dummy;
        self.local_hub = None;
        self.room = None;
    }

    /// Installs local wireless play for instance `inst`.
    ///
    /// Pass `hub` to join an existing local session (from another
    /// selector's [`MpInterfaceSelector::local_hub`]), or `None` to start a
    /// fresh one. The instance is registered with the hub immediately, so
    /// it will see every frame sent from now on.
    pub fn set_local(&mut self, nds: &mut NDS, inst: u8, hub: Option<Arc<LocalMpHub>>) {
        let hub = hub.unwrap_or_else(|| Arc::new(LocalMpHub::new()));
        let mut transport = MpInterfaceTransport::new(LocalMp::from_hub(Arc::clone(&hub)), inst);
        // `begin` is what adds this instance to the hub's connected
        // bitmask; the Wi-Fi hardware only calls it on power-on, which may
        // already have happened.
        transport.begin();
        nds.set_mp_transport(Some(Box::new(transport)));

        self.kind = MpInterfaceType::Local;
        self.local_hub = Some(hub);
        self.room = None;
    }

    /// Installs LAN play, taking over a [`Room`] built by
    /// [`Room::host`] or [`Room::join`].
    ///
    /// Returns the room handle for convenience; the same value stays
    /// available from [`MpInterfaceSelector::room`].
    pub fn set_lan(&mut self, nds: &mut NDS, room: Room) -> RoomHandle {
        let Room { handle, transport } = room;
        nds.set_mp_transport(Some(Box::new(transport)));

        self.kind = MpInterfaceType::Lan;
        self.local_hub = None;
        self.room = Some(handle.clone());
        handle
    }
}
