//! [`NetLink`]: one console's seat on the melonDS MP medium.

use std::time::{Duration, Instant};

use ds_peripherals::wifi::{MpLink, MpRecv};
use melonds_net::mp::{self, MpHandle, Recv};

/// A console's connection to the melonDS MP backend, for
/// `Emulator::set_mp_link`.
///
/// # Stall budget
///
/// An MP client waits for its host's frames (`recv_host_packet`) and a host
/// for its clients' replies (`recv_replies`), each wait bounded by the
/// backend's receive timeout (melonDS: 25 ms). That keeps the consoles in
/// lockstep, but when the peer stops running (paused, window minimised) every
/// 8 µs Wi-Fi step waits the full timeout again and the frame never ends.
///
/// With a budget, waiting stops once the waits of one emulated frame add up
/// to it: the rest of the frame polls without blocking, so the emulator (and
/// a frontend running it on its UI thread) keeps going and the game sees the
/// silence as a communication error, as real hardware would. `None` waits
/// like melonDS, which is right when each console has its own thread.
#[derive(Debug)]
pub struct NetLink {
    handle: MpHandle,
    budget: Option<Duration>,
    left: Duration,
    pump: bool,
}

impl NetLink {
    /// Console `instance` (unique within this process, `< MAX_INSTANCES`;
    /// `0` for a process running one console), with no stall budget and
    /// pumping the backend every frame.
    ///
    /// # Panics
    /// If `instance` is not below [`crate::MAX_INSTANCES`].
    pub fn new(instance: usize) -> Self {
        Self { handle: MpHandle::new(instance), budget: None, left: Duration::ZERO, pump: true }
    }

    /// Bounds the time spent waiting for peers per emulated frame (see the
    /// type docs).
    #[must_use]
    pub const fn with_stall_budget(mut self, budget: Option<Duration>) -> Self {
        self.budget = budget;
        self
    }

    /// Whether [`MpLink::new_frame`] calls [`crate::process`]. Turn it off
    /// for all but one of several consoles in one process.
    #[must_use]
    pub const fn with_pump(mut self, pump: bool) -> Self {
        self.pump = pump;
        self
    }

    /// The melonDS instance number.
    pub const fn instance(&self) -> usize {
        self.handle.instance()
    }

    /// Runs a blocking receive within the stall budget.
    fn bounded<T>(&mut self, recv: impl FnOnce(MpHandle) -> T) -> T {
        if self.budget.is_none() {
            return recv(self.handle);
        }
        if self.left.is_zero() {
            let timeout = mp::recv_timeout();
            mp::set_recv_timeout(0);
            let out = recv(self.handle);
            mp::set_recv_timeout(timeout);
            return out;
        }
        let start = Instant::now();
        let out = recv(self.handle);
        self.left = self.left.saturating_sub(start.elapsed());
        out
    }
}

const fn convert(recv: Recv) -> MpRecv {
    match recv {
        Recv::Empty => MpRecv::Empty,
        Recv::Frame { len, timestamp } => MpRecv::Frame { len, timestamp },
        Recv::HostGone => MpRecv::HostGone,
    }
}

impl MpLink for NetLink {
    fn begin(&mut self) {
        log::debug!("mp: console {} on the air", self.instance());
        self.handle.begin();
    }

    fn end(&mut self) {
        log::debug!("mp: console {} off the air", self.instance());
        self.handle.end();
    }

    fn new_frame(&mut self) {
        if let Some(budget) = self.budget {
            self.left = budget;
        }
        if self.pump {
            crate::process();
        }
    }

    fn send_packet(&mut self, frame: &[u8], timestamp: u64) {
        self.handle.send_packet(frame, timestamp);
    }

    fn send_cmd(&mut self, frame: &[u8], timestamp: u64) {
        self.handle.send_cmd(frame, timestamp);
    }

    fn send_reply(&mut self, frame: &[u8], timestamp: u64, aid: u16) {
        self.handle.send_reply(frame, timestamp, aid);
    }

    fn send_ack(&mut self, frame: &[u8], timestamp: u64) {
        self.handle.send_ack(frame, timestamp);
    }

    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        convert(self.handle.recv_packet(buf))
    }

    fn recv_host_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        convert(self.bounded(|h| h.recv_host_packet(buf)))
    }

    fn recv_replies(&mut self, buf: &mut [u8], timestamp: u64, aidmask: u16) -> u16 {
        self.bounded(|h| h.recv_replies(buf, timestamp, aidmask))
    }
}
