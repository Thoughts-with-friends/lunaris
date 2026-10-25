//! A running count of what each console has sent and received.

/// A running count of what each console has sent and received, which is what
/// the diagnostics window reports.
#[derive(Clone, Copy, Default)]
pub struct Counters {
    pub sent_generic: u64,
    pub sent_cmd: u64,
    pub sent_reply: u64,
    pub sent_ack: u64,
    pub recv_generic: u64,
    pub recv_cmd: u64,
    pub recv_reply: u64,
    /// Replies dropped for being older than the host's round.
    pub stale_replies: u64,
    /// The newest wifi clock this console reported.
    pub clock: u64,
    /// The last AID mask `recv_replies` returned, so a host that is asking but
    /// hearing nothing is visible.
    pub last_reply_mask: u16,
}
