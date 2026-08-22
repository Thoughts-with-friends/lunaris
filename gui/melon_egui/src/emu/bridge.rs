//! The `melonds::Host` a console is built with: where its save goes, why it
//! stopped, and which airwaves it is on.

use super::*;

/// The newest backup-memory image the core has produced, waiting to be written.
pub(crate) struct SaveSink {
    pub(crate) path: PathBuf,
    pub(crate) pending: Mutex<Option<Vec<u8>>>,
}

/// The core's view of the host. Holds only what a callback needs; the front end
/// keeps its own [`Arc`] to the same sink.
pub(crate) struct HostBridge {
    pub(crate) saves: Arc<SaveSink>,
    /// Where `signal_stop` leaves what it was told, for the front end to read
    /// once `run_frame` has returned.
    pub(crate) stop: Arc<Mutex<Option<StopReason>>>,
    /// This console's place on the shared airwaves, when it has one. Without it
    /// every MP hook keeps the trait's default — an unlinked console.
    pub(crate) mp: Option<crate::mp::Client>,
    pub(crate) network: Option<Box<dyn melonds::Host>>,
}

impl melonds::Host for HostBridge {
    /// `data` is the whole backup image, so the offset/length hint of *which*
    /// bytes moved is not needed to keep the file correct — only to write less
    /// than all of it, which this front end does not bother with.
    fn write_save(&self, data: &[u8], _writeoffset: u32, _writelen: u32) {
        *self.saves.pending.lock().unwrap() = Some(data.to_vec());
    }

    /// melonDS is stopping this console. Recorded rather than acted on: the
    /// call arrives from inside `run_frame`, and the front end reads it out
    /// once that has returned — see [`Emu::stop_reason`].
    fn signal_stop(&self, reason: i32) {
        *self.stop.lock().unwrap() = Some(StopReason::from_core(reason));
    }

    // The MP hooks are pure forwarding: the airwaves are shared state, so the
    // interesting behaviour lives in `crate::mp` rather than here.

    fn mp_begin(&self) {
        if let Some(network) = &self.network {
            network.mp_begin();
            return;
        }
        if let Some(mp) = &self.mp {
            mp.mp_begin();
        }
    }

    fn mp_end(&self) {
        if let Some(network) = &self.network {
            network.mp_end();
            return;
        }
        if let Some(mp) = &self.mp {
            mp.mp_end();
        }
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        if let Some(network) = &self.network {
            return network.mp_send_packet(data, timestamp);
        }
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_packet(data, timestamp))
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        if let Some(network) = &self.network {
            return network.mp_send_cmd(data, timestamp);
        }
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_cmd(data, timestamp))
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        if let Some(network) = &self.network {
            return network.mp_send_reply(data, timestamp, aid);
        }
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_reply(data, timestamp, aid))
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        if let Some(network) = &self.network {
            return network.mp_send_ack(data, timestamp);
        }
        self.mp.as_ref().map_or(data.len() as i32, |mp| mp.mp_send_ack(data, timestamp))
    }

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        if let Some(network) = &self.network {
            return network.mp_recv_packet(data, now, timestamp);
        }
        self.mp.as_ref().map_or(Some(0), |mp| mp.mp_recv_packet(data, now, timestamp))
    }

    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        if let Some(network) = &self.network {
            return network.mp_recv_host_packet(data, now, timestamp);
        }
        self.mp.as_ref().and_then(|mp| mp.mp_recv_host_packet(data, now, timestamp))
    }

    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        if let Some(network) = &self.network {
            return network.mp_recv_replies(data, now, timestamp, aidmask);
        }
        self.mp.as_ref().map_or(0, |mp| mp.mp_recv_replies(data, now, timestamp, aidmask))
    }

    fn mp_clock(&self, now: u64) {
        if let Some(network) = &self.network {
            network.mp_clock(now);
            return;
        }
        if let Some(mp) = &self.mp {
            mp.mp_clock(now);
        }
    }
}
