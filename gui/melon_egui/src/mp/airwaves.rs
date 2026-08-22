//! The shared medium every console in this process sits on.

use super::*;

/// One console's queues.
#[derive(Default)]
pub(crate) struct Mailbox {
    pub(crate) packets: VecDeque<Packet>,
    pub(crate) replies: VecDeque<Packet>,
    pub(crate) counters: Counters,
    pub(crate) connected: bool,
    /// The console whose CMD frame this one last *received*, which is what
    /// melonDS's `LastHostID` is: it is set on receive, per instance, so that
    /// "my host has gone" is answered from what this console has actually
    /// heard rather than from who spoke last on the medium.
    pub(crate) last_host: Option<usize>,
    /// When this console last did anything on the air. `None` until it does.
    /// See [`PEER_TIMEOUT`].
    pub(crate) active: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) boxes: Vec<Mailbox>,
    /// A short rolling history for the diagnostics window, newest last.
    pub(crate) log: VecDeque<Event>,
}

/// The shared medium. Cheap to clone; every console holds one.
///
/// The condvar is what melonDS's per-instance semaphores are: a sender wakes
/// whoever is waiting for something to arrive. One for the medium rather than
/// one per console — with two consoles the difference is a spurious wake-up
/// nobody notices, and the waiters re-check what they are waiting for anyway.
#[derive(Clone)]
pub struct Airwaves(pub(crate) Arc<(Mutex<Shared>, Condvar)>);

impl Default for Airwaves {
    fn default() -> Self {
        Self::new()
    }
}

impl Airwaves {
    pub fn new() -> Self {
        let mut shared = Shared::default();
        shared.boxes.resize_with(MAX_INSTANCES, Mailbox::default);
        Self(Arc::new((Mutex::new(shared), Condvar::new())))
    }

    /// A handle for console `instance`, to hand to [`crate::emu::Emu`].
    pub fn client(&self, instance: usize) -> Client {
        Client { airwaves: self.clone(), instance }
    }

    /// Per-console counters, for the diagnostics window.
    pub fn counters(&self) -> Vec<Counters> {
        let shared = self.0.0.lock().unwrap();
        shared.boxes.iter().map(|b| b.counters).collect()
    }

    /// Which consoles have called `mp_begin` and not `mp_end`.
    pub fn connected(&self) -> Vec<bool> {
        let shared = self.0.0.lock().unwrap();
        shared.boxes.iter().map(|b| b.connected).collect()
    }

    /// The rolling traffic log, oldest first.
    pub fn log(&self) -> Vec<Event> {
        let shared = self.0.0.lock().unwrap();
        shared.log.iter().cloned().collect()
    }

    pub fn clear_log(&self) {
        self.0.0.lock().unwrap().log.clear();
    }

    /// The bitmask of connected consoles, as melonDS's `ConnectedBitmask`.
    pub(crate) fn connected_mask(shared: &Shared) -> u16 {
        shared.boxes.iter().enumerate().fold(
            0u16,
            |mask, (i, b)| {
                if b.connected { mask | (1 << i) } else { mask }
            },
        )
    }
}

/// How many consoles can share these airwaves. Two is what "Launch new
/// instance" opens; the AID bookkeeping below allows more.
pub const MAX_INSTANCES: usize = 16;
