//! The datagram layouts, in one place.
//!
//! Every datagram starts with [`MAGIC`] and a [`Kind`]. Nothing here allocates
//! a header of its own or reads a field by hand — a layout that is written down
//! twice is a layout that will disagree with itself.

use super::MAX_DATAGRAM;

/// Identifies this protocol's datagrams.
pub const MAGIC: &[u8; 4] = b"MRD1";

/// `MAGIC` plus the kind byte, which every datagram begins with.
pub const PREFIX: usize = 5;

/// What a datagram is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Client announcing itself.
    Hello = 0,
    /// Host accepting.
    Welcome = 1,
    /// Some of a frame's tiles. Independently applicable.
    Video = 2,
    /// A run of interleaved stereo samples, at the rate named in its header.
    Audio = 3,
    /// The remote player's buttons and stylus, as a whole state.
    Input = 4,
    /// Latency probe, carrying the sender's wall clock in microseconds.
    Ping = 5,
    /// Latency probe echo, carrying the `Ping`'s value untouched.
    Pong = 6,
    /// "Are you ready?" — sent outside any session by a machine that has not
    /// joined yet, carrying its wall clock like a `Ping`. See [`super::probe`].
    Probe = 7,
    /// The answer to a `Probe`: its stamp, then one [`PeerState`] byte.
    ProbeReply = 8,
}

impl Kind {
    #[must_use]
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Hello),
            1 => Some(Self::Welcome),
            2 => Some(Self::Video),
            3 => Some(Self::Audio),
            4 => Some(Self::Input),
            5 => Some(Self::Ping),
            6 => Some(Self::Pong),
            7 => Some(Self::Probe),
            8 => Some(Self::ProbeReply),
            _ => None,
        }
    }
}

/// Start a datagram of `kind`, sized so an ordinary one never reallocates.
#[must_use]
pub fn header(kind: Kind) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAX_DATAGRAM);
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes
}

/// Read a datagram's kind, refusing anything that is not ours.
#[must_use]
pub fn kind_of(bytes: &[u8]) -> Option<Kind> {
    (bytes.len() >= PREFIX && &bytes[..4] == MAGIC).then(|| Kind::from_wire(bytes[4])).flatten()
}

// -- video -------------------------------------------------------------------

/// A video datagram's header: the prefix, the frame sequence, the tile count.
pub const VIDEO_HEADER: usize = PREFIX + 4 + 2;

/// Begin a video datagram. The tile count is filled in by [`finish_video`] once
/// it is known.
#[must_use]
pub fn begin_video(frame_seq: u32) -> Vec<u8> {
    let mut bytes = header(Kind::Video);
    bytes.extend_from_slice(&frame_seq.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

/// Write the tile count into a finished video datagram.
pub fn finish_video(bytes: &mut [u8], tiles: u16) {
    bytes[9..11].copy_from_slice(&tiles.to_le_bytes());
}

/// Read a video datagram's frame sequence and tile count.
#[must_use]
pub fn read_video_header(bytes: &[u8]) -> Option<(u32, u16)> {
    if bytes.len() < VIDEO_HEADER || kind_of(bytes) != Some(Kind::Video) {
        return None;
    }
    let seq = u32::from_le_bytes(bytes[5..9].try_into().ok()?);
    let tiles = u16::from_le_bytes([bytes[9], bytes[10]]);
    Some((seq, tiles))
}

// -- audio -------------------------------------------------------------------

/// An audio datagram's header: the prefix, the sequence, the sample rate.
///
/// The rate travels with every datagram rather than being agreed once, so a
/// client never has to be configured to match and a host may change it
/// mid-session without a renegotiation.
pub const AUDIO_HEADER: usize = PREFIX + 4 + 4;

/// Build an audio datagram from interleaved stereo `i16` at `rate` Hz.
#[must_use]
pub fn encode_audio(seq: u32, rate: u32, samples: &[i16]) -> Vec<u8> {
    let mut bytes = header(Kind::Audio);
    bytes.extend_from_slice(&seq.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Read an audio datagram, returning its rate and its samples.
#[must_use]
pub fn decode_audio(bytes: &[u8]) -> Option<(u32, Vec<i16>)> {
    if bytes.len() < AUDIO_HEADER || kind_of(bytes) != Some(Kind::Audio) {
        return None;
    }
    let rate = u32::from_le_bytes(bytes[9..13].try_into().ok()?);
    let samples =
        bytes[AUDIO_HEADER..].as_chunks::<2>().0.iter().map(|pair| i16::from_le_bytes(*pair));
    Some((rate, samples.collect()))
}

// -- input -------------------------------------------------------------------

/// The remote player's controls, as a whole state.
///
/// Sent every repaint whether or not anything changed, and never as a change.
/// Over UDP the datagram carrying "the button came up" is the one that gets
/// lost, and a differential protocol would then hold the button down for good.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Input {
    /// The `melonds::keys` bitmask.
    pub keys: u32,
    /// Where the stylus is, or `None` for lifted.
    pub touch: Option<(u16, u16)>,
    /// Which sample this is, so an overtaking datagram cannot un-press a
    /// button that a newer one pressed.
    pub seq: u32,
}

/// An input datagram's length: the prefix, sequence, keys, x, y, and the
/// stylus-down flag.
pub const INPUT_LEN: usize = PREFIX + 4 + 4 + 2 + 2 + 1;

#[must_use]
pub fn encode_input(input: Input) -> Vec<u8> {
    let mut bytes = header(Kind::Input);
    bytes.extend_from_slice(&input.seq.to_le_bytes());
    bytes.extend_from_slice(&input.keys.to_le_bytes());
    let (x, y, down) = match input.touch {
        Some((x, y)) => (x, y, 1u8),
        None => (0, 0, 0),
    };
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes.push(down);
    bytes
}

#[must_use]
pub fn decode_input(bytes: &[u8]) -> Option<Input> {
    if bytes.len() < INPUT_LEN || kind_of(bytes) != Some(Kind::Input) {
        return None;
    }
    let seq = u32::from_le_bytes(bytes[5..9].try_into().ok()?);
    let keys = u32::from_le_bytes(bytes[9..13].try_into().ok()?);
    let x = u16::from_le_bytes([bytes[13], bytes[14]]);
    let y = u16::from_le_bytes([bytes[15], bytes[16]]);
    let touch = (bytes[17] != 0).then_some((x, y));
    Some(Input { keys, touch, seq })
}

// -- probes ------------------------------------------------------------------

/// A `Ping` carrying the sender's wall clock.
#[must_use]
pub fn encode_ping(now_micros: u64) -> Vec<u8> {
    let mut bytes = header(Kind::Ping);
    bytes.extend_from_slice(&now_micros.to_le_bytes());
    bytes
}

/// The `Pong` that answers `ping`, echoing its stamp untouched.
#[must_use]
pub fn echo_ping(ping: &[u8]) -> Vec<u8> {
    let mut bytes = header(Kind::Pong);
    bytes.extend_from_slice(&ping[PREFIX..ping.len().min(PREFIX + 8)]);
    bytes
}

/// The stamp a `Pong` is carrying back.
#[must_use]
pub fn read_pong(bytes: &[u8]) -> Option<u64> {
    (bytes.len() >= PREFIX + 8)
        .then(|| bytes[PREFIX..PREFIX + 8].try_into().ok().map(u64::from_le_bytes))
        .flatten()
}

/// What a host says about itself in a [`Kind::ProbeReply`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeerState {
    /// Hosting and waiting for a client: joining now will succeed.
    Waiting = 0,
    /// Hosting, but already streaming to someone else.
    Busy = 1,
}

/// A `Probe` carrying the sender's wall clock.
#[must_use]
pub fn encode_probe(now_micros: u64) -> Vec<u8> {
    let mut bytes = header(Kind::Probe);
    bytes.extend_from_slice(&now_micros.to_le_bytes());
    bytes
}

/// The `ProbeReply` that answers `probe` with `state`, echoing its stamp.
#[must_use]
pub fn answer_probe(probe: &[u8], state: PeerState) -> Vec<u8> {
    let mut bytes = header(Kind::ProbeReply);
    let mut stamp = [0u8; 8];
    let carried = probe.get(PREFIX..probe.len().min(PREFIX + 8)).unwrap_or(&[]);
    stamp[..carried.len()].copy_from_slice(carried);
    bytes.extend_from_slice(&stamp);
    bytes.push(state as u8);
    bytes
}

/// The stamp and state a `ProbeReply` carries.
#[must_use]
pub fn read_probe_reply(bytes: &[u8]) -> Option<(u64, PeerState)> {
    if bytes.len() < PREFIX + 9 || kind_of(bytes) != Some(Kind::ProbeReply) {
        return None;
    }
    let stamp = u64::from_le_bytes(bytes[PREFIX..PREFIX + 8].try_into().ok()?);
    let state = match bytes[PREFIX + 8] {
        0 => PeerState::Waiting,
        1 => PeerState::Busy,
        _ => return None,
    };
    Some((stamp, state))
}

/// The sender's wall clock in microseconds, for the latency probe.
///
/// Only ever differenced against another reading **on the same machine** — a
/// `Ping` is timed by whoever sent it, from its own echo — so the two clocks do
/// not have to agree.
#[must_use]
pub fn wall_clock_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_micros().min(u128::from(u64::MAX)) as u64)
}

#[cfg(test)]
mod tests {
    use super::{
        Input, Kind, PeerState, answer_probe, decode_audio, decode_input, echo_ping, encode_audio,
        encode_input, encode_ping, encode_probe, kind_of, read_pong, read_probe_reply,
        read_video_header,
    };

    #[test]
    fn a_probe_reply_carries_the_stamp_and_the_state() {
        let probe = encode_probe(0xDEAD_BEEF);
        assert_eq!(kind_of(&probe), Some(Kind::Probe));
        for state in [PeerState::Waiting, PeerState::Busy] {
            assert_eq!(read_probe_reply(&answer_probe(&probe, state)), Some((0xDEAD_BEEF, state)));
        }
        assert_eq!(read_probe_reply(&encode_ping(1)), None, "a ping is not a reply");
    }

    /// A lifted stylus that decoded as `Some((0, 0))` would drag the pointer to
    /// the corner of the screen every time the player let go.
    #[test]
    fn input_round_trips_including_a_lifted_stylus() {
        for input in [
            Input { keys: 0, touch: None, seq: 1 },
            Input { keys: 0x3FF, touch: Some((128, 96)), seq: 2 },
            Input { keys: 1, touch: Some((0, 0)), seq: u32::MAX },
        ] {
            assert_eq!(decode_input(&encode_input(input)), Some(input));
        }
        assert!(decode_input(b"MRD1\x04short").is_none());
    }

    /// The rate has to survive, or the client resamples against the wrong
    /// number and everything is pitched.
    #[test]
    fn audio_round_trips_with_its_rate() {
        let samples: Vec<i16> = (0..64).map(|i| i * 100 - 3_200).collect();
        let (rate, back) = decode_audio(&encode_audio(7, 24_000, &samples)).expect("valid audio");
        assert_eq!(rate, 24_000);
        assert_eq!(back, samples);
    }

    #[test]
    fn a_ping_comes_back_with_its_stamp_untouched() {
        let ping = encode_ping(0x0123_4567_89AB_CDEF);
        let pong = echo_ping(&ping);
        assert_eq!(kind_of(&pong), Some(Kind::Pong));
        assert_eq!(read_pong(&pong), Some(0x0123_4567_89AB_CDEF));
    }

    #[test]
    fn a_foreign_or_truncated_datagram_is_refused() {
        assert!(kind_of(b"nope").is_none());
        assert!(kind_of(&[]).is_none());
        assert!(kind_of(b"MRD1\x7F").is_none(), "an unknown kind");
        assert!(read_video_header(b"MRD1\x02").is_none());
        assert!(decode_audio(b"MRD1\x03tiny").is_none());
        // Right length, wrong kind.
        assert!(decode_input(&encode_audio(1, 24_000, &[0; 8])).is_none());
    }
}
