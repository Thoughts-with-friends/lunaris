//! The datagram layout, and the batching that shares one between frames.

use super::*;

// -- wire format -------------------------------------------------------------

/// Append one frame to `bytes`. Several frames may share a datagram; the
/// decoder loops until the buffer is consumed, which is what makes
/// [`Coalescer`] possible without a second envelope layer.
pub(crate) fn encode_into(
    bytes: &mut Vec<u8>,
    kind: Kind,
    aid: u16,
    timestamp: u64,
    payload: &[u8],
) {
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes.extend_from_slice(&aid.to_le_bytes());
    bytes.extend_from_slice(&timestamp.to_le_bytes());
    // Sequence is stamped per datagram, not per frame, and filled in by
    // `Peer::transmit`; a placeholder keeps the layout fixed-width.
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.extend_from_slice(payload);
}

/// Overwrite every frame header's sequence field in a finished datagram.
///
/// Every frame in one datagram shares its sequence number, because the
/// duplicate rejection this feeds is about datagrams: a redundant copy repeats
/// the whole datagram, not one frame of it.
pub(crate) fn stamp_sequence(bytes: &mut [u8], sequence: u32) {
    let mut at = 0;
    while at + HEADER_LEN <= bytes.len() {
        let len = u16::from_le_bytes([bytes[at + 19], bytes[at + 20]]) as usize;
        bytes[at + 15..at + 19].copy_from_slice(&sequence.to_le_bytes());
        at += HEADER_LEN + len;
    }
}

/// Read every frame out of one datagram, with the sequence they share.
///
/// Returns `None` for a datagram that is not ours or is malformed, which is not
/// an error worth reporting: a UDP port takes whatever is sent to it.
pub(crate) fn decode(bytes: &[u8]) -> Option<(u32, Vec<Frame>)> {
    let mut frames = Vec::new();
    let mut sequence = None;
    let mut at = 0;
    while at + HEADER_LEN <= bytes.len() {
        if &bytes[at..at + 4] != MAGIC {
            return None;
        }
        let kind = Kind::from_wire(bytes[at + 4])?;
        let aid = u16::from_le_bytes([bytes[at + 5], bytes[at + 6]]);
        let timestamp = u64::from_le_bytes(bytes[at + 7..at + 15].try_into().ok()?);
        let seq = u32::from_le_bytes(bytes[at + 15..at + 19].try_into().ok()?);
        let len = u16::from_le_bytes([bytes[at + 19], bytes[at + 20]]) as usize;
        if len > MAX_PAYLOAD || at + HEADER_LEN + len > bytes.len() {
            return None;
        }
        let payload = bytes[at + HEADER_LEN..at + HEADER_LEN + len].to_vec();
        sequence.get_or_insert(seq);
        frames.push(Frame { kind, aid, timestamp, payload });
        at += HEADER_LEN + len;
    }
    // A trailing partial frame means a truncated datagram; the frames before it
    // are still good, but a datagram with nothing in it is not.
    (!frames.is_empty()).then(|| (sequence.unwrap_or(0), frames))
}
