//! Sharing one melonDS `Host` between the console that owns it and the pane
//! that reports on it.

/// A LAN link that finished its handshake on the connection thread, on its way
/// to being handed to a console.
///
/// Carries the link's own measurement handles alongside the transport, because
/// `Box<dyn melonds::Host>` erases them and the front end needs both: the stats
/// for the Wireless pane, and the pace for [`MelonEgui::advance`].
pub(crate) struct LanConnection {
    pub(crate) host: Box<dyn melonds::Host>,
    pub(crate) local_addr: String,
    pub(crate) remote_addr: String,
    /// Reads the live link counters. `None` would mean a transport with no
    /// measurement, which this front end no longer has.
    pub(crate) stats: Box<dyn Fn() -> crate::lan::LinkStats + Send>,
    pub(crate) pace: crate::lan::LinkPace,
}

/// Lets a link be both the console's `Host` and the pane's counter source.
///
/// `Nds::new` takes ownership of a `Box<dyn Host>`, but the Wireless pane has
/// to keep reading the same link's counters for as long as it is up. Sharing
/// the transport behind an `Arc` is the whole of the trick; every method simply
/// forwards.
pub(crate) struct ArcHost<T>(pub(crate) std::sync::Arc<T>);

impl<T: melonds::Host + Sync> melonds::Host for ArcHost<T> {
    fn write_save(&self, data: &[u8], writeoffset: u32, writelen: u32) {
        self.0.write_save(data, writeoffset, writelen);
    }

    fn signal_stop(&self, reason: i32) {
        self.0.signal_stop(reason);
    }

    fn mp_begin(&self) {
        self.0.mp_begin();
    }

    fn mp_end(&self) {
        self.0.mp_end();
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        self.0.mp_send_packet(data, timestamp)
    }

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.0.mp_recv_packet(data, now, timestamp)
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        self.0.mp_send_cmd(data, timestamp)
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        self.0.mp_send_reply(data, timestamp, aid)
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        self.0.mp_send_ack(data, timestamp)
    }

    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.0.mp_recv_host_packet(data, now, timestamp)
    }

    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        self.0.mp_recv_replies(data, now, timestamp, aidmask)
    }

    fn mp_clock(&self, now: u64) {
        self.0.mp_clock(now);
    }
}
