//! The knobs behind Remote Desktop mode, and the bounds they are held to.

use super::{CONSOLE_SAMPLE_RATE, NATIVE_FPS};

/// How Remote Desktop behaves, persisted in the instance's `settings.json`.
///
/// Every field is a bandwidth-against-smoothness trade, and every default is
/// chosen for a VPN rather than for a LAN: the whole mode exists because a VPN
/// was the problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// How many *sent* frames a complete rolling refresh takes.
    ///
    /// This is the whole of the loss recovery: a tile that was sent and lost is
    /// repainted within this many sent frames whatever else happens. Lower
    /// recovers faster and costs bandwidth on a still picture.
    pub refresh_period: u8,

    /// The most frames a second that are put on the wire.
    ///
    /// The console still runs at 59.83; this is only how often its picture is
    /// captured. 30 halves the video bandwidth for a cost most players do not
    /// notice, and — this is the part that matters — **skipping a frame adds no
    /// latency at all**: a datagram goes out the instant it is encoded, so what
    /// is traded is smoothness, never immediacy.
    pub max_video_fps: u8,

    /// The fewest frames a second the adaptive skip will fall to.
    ///
    /// A floor, so a link that is briefly terrible does not leave the picture
    /// frozen. Below about 8 the screen stops reading as motion.
    pub min_video_fps: u8,

    /// The video bit rate the adaptive skip aims to stay under, in kbit/s.
    ///
    /// Measured over the last second of real datagrams, headers included. When
    /// it is exceeded the frame interval grows; when there is room to spare it
    /// shrinks back towards [`Self::max_video_fps`].
    pub max_bitrate_kbps: u32,

    /// Whether the remote player hears the console.
    pub audio: bool,

    /// The rate the sound travels at, in Hz.
    ///
    /// The console produces 48 kHz; halving that halves the 1.5 Mbit/s the
    /// sound would otherwise cost, and the client resamples back up before the
    /// sound card ever sees it. See [`super::audio`]. At or above 48 kHz this
    /// does nothing.
    pub audio_rate: u32,

    /// How much audio may be in flight before the oldest is dropped, in
    /// milliseconds.
    ///
    /// Audio that is queued is audio that is late, and a queue that is never
    /// trimmed grows without bound — the sound drifts further behind the
    /// picture for as long as the session lasts. Dropping is audible once;
    /// drifting is audible forever.
    pub max_audio_lag_ms: u16,

    /// The UDP port the host binds and the client connects to.
    pub port: u16,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            refresh_period: 8,
            max_video_fps: 30,
            min_video_fps: 10,
            max_bitrate_kbps: 3_000,
            audio: true,
            audio_rate: CONSOLE_SAMPLE_RATE / 2,
            max_audio_lag_ms: 120,
            port: 7065,
        }
    }
}

impl Tuning {
    /// Clamp every field to something the protocol can honour, so a
    /// hand-edited `settings.json` cannot put a session in a state the UI could
    /// not produce.
    pub fn normalize(&mut self) {
        let defaults = Self::default();
        self.refresh_period = self.refresh_period.clamp(1, 60);
        self.max_video_fps = self.max_video_fps.clamp(5, 60);
        self.min_video_fps = self.min_video_fps.clamp(5, self.max_video_fps);
        self.max_bitrate_kbps = self.max_bitrate_kbps.clamp(200, 100_000);
        // 4 kHz is a telephone; below that the sound is not worth the bytes.
        self.audio_rate = self.audio_rate.clamp(4_000, CONSOLE_SAMPLE_RATE);
        self.max_audio_lag_ms = self.max_audio_lag_ms.clamp(20, 1_000);
        if self.port == 0 {
            self.port = defaults.port;
        }
    }

    /// How many emulated frames pass between two sent frames at
    /// [`Self::max_video_fps`] — the *fastest* the picture is ever sent.
    #[must_use]
    pub fn fastest_interval(&self) -> u32 {
        interval_for(f64::from(self.max_video_fps))
    }

    /// The same at [`Self::min_video_fps`] — the slowest the adaptive skip will
    /// fall to.
    #[must_use]
    pub fn slowest_interval(&self) -> u32 {
        interval_for(f64::from(self.min_video_fps))
    }

    /// How many sample pairs may be queued at the transport rate before the
    /// oldest are dropped.
    #[must_use]
    pub fn audio_backlog_pairs(&self) -> usize {
        (self.audio_rate as usize * usize::from(self.max_audio_lag_ms)) / 1000
    }
}

/// Emulated frames per sent frame, for a target rate. Never zero.
fn interval_for(fps: f64) -> u32 {
    if fps <= 0.0 {
        return 1;
    }
    (NATIVE_FPS / fps).round().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    use super::Tuning;
    use crate::remote::CONSOLE_SAMPLE_RATE;

    #[test]
    fn the_defaults_halve_the_audio_and_the_frame_rate() {
        let tuning = Tuning::default();
        assert_eq!(tuning.audio_rate, CONSOLE_SAMPLE_RATE / 2);
        assert_eq!(tuning.fastest_interval(), 2, "30 fps is every other emulated frame");
        assert_eq!(tuning.slowest_interval(), 6, "10 fps is every sixth");
    }

    #[test]
    fn clamping_a_hand_edited_file_leaves_it_usable() {
        let mut tuning = Tuning {
            refresh_period: 0,
            max_video_fps: 200,
            // Below the ceiling, and also nonsense on its own.
            min_video_fps: 0,
            max_bitrate_kbps: 1,
            audio: true,
            audio_rate: 10,
            max_audio_lag_ms: 5,
            port: 0,
        };
        tuning.normalize();
        assert_eq!(tuning.refresh_period, 1);
        assert_eq!(tuning.max_video_fps, 60);
        assert_eq!(tuning.min_video_fps, 5);
        assert_eq!(tuning.max_bitrate_kbps, 200);
        assert_eq!(tuning.audio_rate, 4_000);
        assert_eq!(tuning.max_audio_lag_ms, 20);
        assert_eq!(tuning.port, Tuning::default().port);
    }

    /// The floor must never end up above the ceiling, or the adaptive skip
    /// would have an empty range to move in.
    #[test]
    fn the_floor_is_pulled_below_the_ceiling() {
        let mut tuning = Tuning { max_video_fps: 15, min_video_fps: 60, ..Tuning::default() };
        tuning.normalize();
        assert!(tuning.min_video_fps <= tuning.max_video_fps);
        assert!(tuning.fastest_interval() <= tuning.slowest_interval());
    }

    #[test]
    fn the_audio_backlog_is_a_real_number_of_pairs() {
        let tuning = Tuning::default();
        // 120 ms at 24 kHz.
        assert_eq!(tuning.audio_backlog_pairs(), 2_880);
    }
}
