//! The medium the Wi-Fi controller transmits on.
//!
//! ```text
//!  WiFi (TX slots, RX ring buffer, MP CMD/reply/ACK sequencing)
//!    |  send_* / recv_*        (12-byte TX/RX header + 802.11 frame)
//!    v
//!  dyn MpLink  -- implemented by `ds_net::NetLink` (melonDS LocalMP / LAN)
//!               or by a test double
//! ```
//!
//! The calls mirror melonDS's `Platform::MP_*` hooks one to one, so the
//! controller code stays a straight port of `Wifi.cpp`. Timestamps are the
//! sender's Wi-Fi microsecond clock (`USTimestamp`).

/// What a receive produced.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpRecv {
    /// Nothing arrived.
    Empty,
    /// `len` bytes were written to the buffer; the sender sent them at
    /// `timestamp`.
    Frame { len: usize, timestamp: u64 },
    /// The MP host this console was a client of has left.
    HostGone,
}

/// A console's connection to the other consoles (melonDS `MPInterface`).
pub trait MpLink: Send + core::fmt::Debug {
    /// The Wi-Fi controller was clocked on (melonDS `MP_Begin`).
    fn begin(&mut self);
    /// The Wi-Fi controller was clocked off (melonDS `MP_End`).
    fn end(&mut self);
    /// Called once at the start of every emulated video frame.
    fn new_frame(&mut self) {}

    /// Ordinary frame (beacon, management, data).
    fn send_packet(&mut self, frame: &[u8], timestamp: u64);
    /// Multiplay host CMD frame.
    fn send_cmd(&mut self, frame: &[u8], timestamp: u64);
    /// Multiplay client reply as `aid`; an empty `frame` means "no reply".
    fn send_reply(&mut self, frame: &[u8], timestamp: u64, aid: u16);
    /// Multiplay host ACK frame.
    fn send_ack(&mut self, frame: &[u8], timestamp: u64);

    /// Non-blocking receive of any frame.
    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv;
    /// Client side: wait (bounded) for the host's next CMD/ACK frame.
    fn recv_host_packet(&mut self, buf: &mut [u8]) -> MpRecv;
    /// Host side: collect the replies to the CMD sent at `timestamp`. Reply
    /// `aid` lands at `buf[(aid - 1) * 1024..]`; returns the AIDs received.
    fn recv_replies(&mut self, buf: &mut [u8], timestamp: u64, aidmask: u16) -> u16;
}
