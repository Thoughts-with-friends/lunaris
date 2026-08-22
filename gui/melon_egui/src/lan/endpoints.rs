//! The host and guest ends, and the `melonds::Host` they present to a console.

use super::*;

// -- the two ends ------------------------------------------------------------

/// The host side of a link: binds a port and waits for one guest.
pub struct LanHost {
    pub(crate) peer: Arc<Peer>,
    pace: LinkPace,
}

/// The guest side of a link: connects to a host and waits for its welcome.
pub struct LanGuest {
    pub(crate) peer: Arc<Peer>,
    pace: LinkPace,
}

impl LanHost {
    /// Bind `bind_addr` and wait for a guest's `HELLO`, answering it.
    ///
    /// Blocks until one arrives, so the caller runs it off the UI thread.
    ///
    /// # Errors
    ///
    /// If the port cannot be bound, or the socket fails while waiting.
    pub fn accept(bind_addr: SocketAddr, tuning: Tuning) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let mut buffer = vec![0u8; HEADER_LEN + MAX_PAYLOAD];
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((len, guest)) => {
                    let Some((_, frames)) = decode(&buffer[..len]) else { continue };
                    if !frames.iter().any(|frame| frame.kind == Kind::Hello) {
                        continue;
                    }
                    let mut welcome = Vec::new();
                    encode_into(&mut welcome, Kind::Welcome, 0, 0, &[]);
                    // Sent before the peer exists, so it is not `transmit`'s
                    // business; three copies because losing the welcome costs
                    // the guest its whole connection attempt.
                    for _ in 0..3 {
                        socket.send_to(&welcome, guest)?;
                    }
                    let (peer, pace) = Peer::start(socket, guest, tuning)?;
                    return Ok(Self { peer, pace });
                }
                Err(ref error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.peer.socket.local_addr()
    }

    /// The guest that connected.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.peer.remote
    }

    /// What the link is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> LinkStats {
        self.peer.stats()
    }

    /// The frame rate handle the front end paces the console to.
    #[must_use]
    pub fn pace(&self) -> LinkPace {
        self.pace.clone()
    }
}

impl LanGuest {
    /// Bind `bind_addr`, announce to `host_addr`, and wait for its welcome.
    ///
    /// Retries the announcement, because on a VPN the first datagram after the
    /// tunnel comes up is the one most likely to be dropped.
    ///
    /// # Errors
    ///
    /// If the port cannot be bound, or no welcome arrives.
    pub fn connect(
        bind_addr: SocketAddr,
        host_addr: SocketAddr,
        tuning: Tuning,
    ) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut hello = Vec::new();
        encode_into(&mut hello, Kind::Hello, 0, 0, &[]);
        let mut buffer = vec![0u8; HEADER_LEN + MAX_PAYLOAD];
        // Ten seconds in total. A tunnel that is still negotiating routinely
        // eats the first second or two, and failing inside that window makes
        // the front end look broken when it is merely early.
        for _ in 0..10 {
            socket.send_to(&hello, host_addr)?;
            match socket.recv_from(&mut buffer) {
                Ok((len, sender)) if sender == host_addr => {
                    let Some((_, frames)) = decode(&buffer[..len]) else { continue };
                    if frames.iter().any(|frame| frame.kind == Kind::Welcome) {
                        let (peer, pace) = Peer::start(socket, host_addr, tuning)?;
                        return Ok(Self { peer, pace });
                    }
                }
                Ok(_) => continue,
                Err(ref error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no answer from {host_addr} after 10 attempts"),
        ))
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.peer.socket.local_addr()
    }

    /// What the link is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> LinkStats {
        self.peer.stats()
    }

    /// The frame rate handle the front end paces the console to.
    #[must_use]
    pub fn pace(&self) -> LinkPace {
        self.pace.clone()
    }
}

/// The `Host` half, identical for both ends: which side of the handshake a
/// console was on does not change how its wireless behaves.
///
/// Every method is a one-line forward to [`Peer`], where the behaviour lives.
/// That split is deliberate: the trait is only available when the `melonds`
/// feature links the core, and the transport's own tests — which are the
/// evidence that any of this helps — must be runnable without it.
#[cfg(feature = "melonds")]
macro_rules! impl_host {
    ($type:ty) => {
        impl melonds::Host for $type {
            fn mp_begin(&self) {
                self.peer.begin();
            }

            fn mp_end(&self) {
                self.peer.end();
            }

            fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
                self.peer.send(Kind::Packet, data, timestamp, 0)
            }

            fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
                self.peer.send(Kind::Cmd, data, timestamp, 0)
            }

            fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
                self.peer.send(Kind::Reply, data, timestamp, aid)
            }

            fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
                self.peer.send(Kind::Ack, data, timestamp, 0)
            }

            fn mp_recv_packet(
                &self,
                data: &mut [u8],
                _now: u64,
                timestamp: &mut u64,
            ) -> Option<i32> {
                self.peer.recv_packet(data, timestamp)
            }

            fn mp_recv_host_packet(
                &self,
                data: &mut [u8],
                _now: u64,
                timestamp: &mut u64,
            ) -> Option<i32> {
                self.peer.recv_host_packet(data, timestamp)
            }

            fn mp_recv_replies(
                &self,
                data: &mut [u8],
                _now: u64,
                timestamp: u64,
                aidmask: u16,
            ) -> u16 {
                self.peer.recv_replies(data, timestamp, aidmask)
            }
        }
    };
}

#[cfg(feature = "melonds")]
impl_host!(LanHost);
#[cfg(feature = "melonds")]
impl_host!(LanGuest);

/// Winding the receive and service threads up is the same on both ends, and has
/// to happen however the link ends — including when a connection attempt is
/// dropped half-built.
macro_rules! impl_drop {
    ($type:ty) => {
        impl Drop for $type {
            fn drop(&mut self) {
                self.peer.shutdown.store(true, Ordering::Relaxed);
            }
        }
    };
}

impl_drop!(LanHost);
impl_drop!(LanGuest);
