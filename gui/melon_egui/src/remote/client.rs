//! The machine that only displays: no cart, no save, no emulation.

use std::{
    io,
    net::{SocketAddr, UdpSocket},
    sync::{Arc, atomic::Ordering},
};

use super::{
    RemoteStats, Tuning,
    session::{self, Session},
    wire,
};

/// The client end of a Remote Desktop session.
pub struct RemoteClient {
    session: Arc<Session>,
}

impl RemoteClient {
    /// Bind `bind_addr`, announce to `host_addr`, and wait for its welcome.
    ///
    /// # Errors
    /// If the port cannot be bound, or no welcome arrives.
    pub fn connect(
        bind_addr: SocketAddr,
        host_addr: SocketAddr,
        mut tuning: Tuning,
    ) -> io::Result<Self> {
        tuning.normalize();
        let socket = UdpSocket::bind(bind_addr)?;
        session::exchange_hello(&socket, host_addr)?;
        Ok(Self { session: Session::start(socket, host_addr, tuning, false)? })
    }

    /// Send this repaint's controls.
    ///
    /// Called at the **start** of a repaint, before any decoding — see
    /// `crate::app`'s client path for why that ordering is worth several
    /// milliseconds of felt latency. Sent every repaint whatever the player is
    /// doing, because the state is whole rather than differential; see
    /// [`wire::Input`].
    pub fn send_input(&self, keys: u32, touch: Option<(u16, u16)>) {
        let seq = self.session.next_seq();
        let datagram = wire::encode_input(wire::Input { keys, touch, seq });
        if self.session.send(&datagram) {
            let counters = &self.session.counters;
            counters.bump(&counters.inputs, 1);
        }
    }

    /// The newest picture, if anything has been painted since the last call.
    pub fn take_screens(&self) -> Option<[Vec<u32>; 2]> {
        self.session.take_screens()
    }

    /// Everything waiting to be played, and the rate it is at.
    ///
    /// The rate is the host's transport rate, not the console's: the caller
    /// hands both to [`crate::audio::Audio::push_at`], whose resampler does the
    /// upsampling on the way to the sound card.
    pub fn take_audio(&self) -> (Vec<i16>, u32) {
        self.session.take_audio()
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.session.local_addr()
    }

    /// The host this end is watching.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.session.remote
    }

    /// What the session is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> RemoteStats {
        self.session.counters.snapshot(true)
    }
}

impl Drop for RemoteClient {
    fn drop(&mut self) {
        self.session.shutdown.store(true, Ordering::Relaxed);
    }
}
