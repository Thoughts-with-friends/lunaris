//! The knobs a slow link needs, and the bounds they are held to.

// -- tuning -----------------------------------------------------------------

/// The knobs a user can turn, persisted in the instance's `settings.json`.
///
/// Defaults are the LAN-friendly end of each range: a link that needs none of
/// this behaves as `melonds::lan` did, only without the fixed 25 ms ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Floor for the reply wait, in milliseconds. Never below this even on a
    /// link that measures faster, so a single fast probe cannot starve a round.
    pub min_budget_ms: u16,
    /// Ceiling for the reply wait, in milliseconds. This is the real limit on
    /// how bad a link the transport will still try to play over: past it the
    /// emulated frame rate is so low the game is unplayable anyway.
    pub max_budget_ms: u16,
    /// How many multiples of the measured jitter are added to the budget on top
    /// of the round trip. Higher rides out a spikier link at the cost of
    /// waiting longer on the frames that *are* lost.
    pub jitter_factor: u8,
    /// How many copies of each reply and CMD go on the wire. 1 disables
    /// redundancy; 2 is the default and removes the great majority of
    /// single-packet losses.
    pub reply_copies: u8,
    /// How long ordinary (non-round) frames may wait to share a datagram, in
    /// milliseconds. 0 disables batching.
    pub batch_window_ms: u16,
    /// Whether the emulated frame rate follows what the link can sustain. Off,
    /// the console runs at 59.83 Hz and drops rounds it cannot service.
    pub pace_to_link: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            min_budget_ms: 8,
            // 250 ms covers a transcontinental VPN. Past that a DS game's own
            // timeouts give up regardless of what this does.
            max_budget_ms: 250,
            jitter_factor: 4,
            reply_copies: 2,
            // Off by default. Batching only ever applies to beacons and the
            // association handshake, and delaying those is the one thing here
            // that could break a link which currently works — while the gain,
            // fewer datagrams on a per-packet-costly tunnel, is unmeasured in
            // operation. It is offered as a knob rather than imposed.
            batch_window_ms: 0,
            pace_to_link: true,
        }
    }
}

impl Tuning {
    /// Clamp every field to a range the transport can actually honour, so a
    /// hand-edited `settings.json` cannot put the link in a state the UI could
    /// not produce.
    pub fn normalize(&mut self) {
        self.min_budget_ms = self.min_budget_ms.clamp(1, 200);
        self.max_budget_ms = self.max_budget_ms.clamp(self.min_budget_ms, 1000);
        self.jitter_factor = self.jitter_factor.clamp(0, 16);
        self.reply_copies = self.reply_copies.clamp(1, 4);
        self.batch_window_ms = self.batch_window_ms.min(50);
    }
}
