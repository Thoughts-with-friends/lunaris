//! The callbacks the melonDS core makes back into this front end.
//!
//! # Overview
//!
//! `melonds::Nds::new` takes a `Box<dyn melonds::Host>`, and the core calls it
//! from inside `run_frame` whenever it needs something from the outside world:
//!
//! | callback        | what happens here                                        |
//! |-----------------|----------------------------------------------------------|
//! | `write_save`    | the newest save image is parked in [`SaveSink`]          |
//! | `signal_stop`   | the reason is parked for [`super::Emu::stop_reason`]     |
//! | `mp_*` (wifi)   | forwarded to the LAN link, else the local airwaves, else |
//! |                 | the trait default (a console with nobody to talk to)     |
//!
//! Nothing is *acted on* here: the call arrives mid-frame, so it only records
//! what happened and the front end reads it once `run_frame` has returned.

use super::*;

/// The newest backup-memory image the core has produced, waiting to be written.
///
/// The core reports every save write as it happens; only the newest image is
/// kept, and [`super::Emu::flush_save`] writes it to disk once a second.
pub(crate) struct SaveSink {
    pub(crate) path: PathBuf,
    pub(crate) pending: Mutex<Option<Vec<u8>>>,
}

/// The `Host` a console is built with.
pub(crate) struct HostBridge {
    pub(crate) saves: Arc<SaveSink>,
    /// Where `signal_stop` leaves its reason.
    pub(crate) stop: Arc<Mutex<Option<StopReason>>>,
    /// This console's seat on the in-process airwaves (local multiplayer).
    pub(crate) mp: Option<crate::mp::Client>,
    /// A LAN link, which takes priority over the seat when present.
    pub(crate) network: Option<Box<dyn melonds::Host>>,
}

/// A wireless back end with nobody on it: every hook keeps the trait default.
struct Unlinked;
impl melonds::Host for Unlinked {}

impl HostBridge {
    /// Where the wireless hooks go: the LAN link, else the local seat, else
    /// nowhere.
    fn wifi(&self) -> &dyn melonds::Host {
        match (&self.network, &self.mp) {
            (Some(network), _) => network.as_ref(),
            (None, Some(mp)) => mp,
            (None, None) => &Unlinked,
        }
    }
}

impl melonds::Host for HostBridge {
    /// `data` is the whole backup image, so the offset/length hint is not
    /// needed to keep the file correct.
    fn write_save(&self, data: &[u8], _writeoffset: u32, _writelen: u32) {
        *self.saves.pending.lock().unwrap() = Some(data.to_vec());
    }

    fn signal_stop(&self, reason: i32) {
        *self.stop.lock().unwrap() = Some(StopReason::from_core(reason));
    }

    fn mp_begin(&self) {
        self.wifi().mp_begin();
    }

    fn mp_end(&self) {
        self.wifi().mp_end();
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        self.wifi().mp_send_packet(data, timestamp)
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        self.wifi().mp_send_cmd(data, timestamp)
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        self.wifi().mp_send_reply(data, timestamp, aid)
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        self.wifi().mp_send_ack(data, timestamp)
    }

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.wifi().mp_recv_packet(data, now, timestamp)
    }

    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.wifi().mp_recv_host_packet(data, now, timestamp)
    }

    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        self.wifi().mp_recv_replies(data, now, timestamp, aidmask)
    }

    fn mp_clock(&self, now: u64) {
        self.wifi().mp_clock(now);
    }
}
