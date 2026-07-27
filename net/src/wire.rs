//! Wire formats: the TCP room-control protocol (§5.3) and the UDP MP-relay
//! packet header (§5.4). See `docs/design/design_lan.md`.

use std::io::{self, Read, Write};

/// Control-channel magic ("LNLN"), checked on every parsed message so a
/// stray connection on the wrong port fails fast instead of silently
/// misparsing.
pub const CONTROL_MAGIC: u32 = 0x4E4C_4E4C;
/// MP-relay magic ("LNMP").
pub const MP_MAGIC: u32 = 0x504D_4E4C;
/// Wire protocol version. Bumped on any incompatible framing change; peers
/// with mismatched versions are rejected rather than guessing.
pub const PROTOCOL_VERSION: u16 = 1;

pub const NAME_LEN: usize = 32;
pub const FINGERPRINT_LEN: usize = 16;

fn write_fixed(out: &mut Vec<u8>, s: &str, len: usize) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(len);
    out.extend_from_slice(&bytes[..n]);
    out.resize(out.len() + (len - n), 0);
}

fn read_fixed(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// One parsed control-channel message (`docs/design/design_lan.md` §5.3).
#[derive(Debug, Clone)]
pub enum ControlMessage {
    Hello {
        player_name: String,
        rom_fingerprint: [u8; FINGERPRINT_LEN],
        mac_suffix: [u8; 3],
        udp_port: u16,
    },
    Welcome {
        player_id: u8,
        max_players: u8,
        room_name: String,
        host_rom_fingerprint: [u8; FINGERPRINT_LEN],
        /// UDP port the host's MP relay is actually listening on. Carried
        /// explicitly rather than assumed equal to the joiner's own
        /// configured `mp_port`, since the two need not match (e.g. two
        /// instances on the same machine for testing, or a host behind
        /// port-mapping).
        host_mp_port: u16,
        link: LinkParams,
    },
    Reject {
        reason: RejectReason,
    },
    PlayerList {
        players: Vec<PlayerRecord>,
    },
    RomChanged {
        rom_fingerprint: [u8; FINGERPRINT_LEN],
    },
    MpReady {
        ready: bool,
    },
    Heartbeat {
        tick: u32,
        sustainable_fps_x10: u16,
        blocked_ms_avg: u16,
        reply_success_x1000: u16,
    },
    LinkParams {
        link: LinkParams,
    },
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    RoomFull,
    VersionMismatch,
    MacCollision,
    Banned,
}

impl RejectReason {
    const fn to_byte(self) -> u8 {
        match self {
            RejectReason::RoomFull => 0,
            RejectReason::VersionMismatch => 1,
            RejectReason::MacCollision => 2,
            RejectReason::Banned => 3,
        }
    }

    const fn from_byte(b: u8) -> Self {
        match b {
            1 => RejectReason::VersionMismatch,
            2 => RejectReason::MacCollision,
            3 => RejectReason::Banned,
            _ => RejectReason::RoomFull,
        }
    }
}

/// One row of a `PlayerList` message.
#[derive(Debug, Clone)]
pub struct PlayerRecord {
    pub id: u8,
    pub name: String,
    pub rom_fingerprint: [u8; FINGERPRINT_LEN],
    pub is_host: bool,
    pub mp_ready: bool,
    pub rtt_ms: u16,
    pub fps_x10: u16,
}

/// Adaptive link parameters broadcast by the host (§9.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkParams {
    pub runahead_us: u32,
    pub recv_timeout_ms: u16,
    pub target_fps_x10: u16,
    pub auto: bool,
}

impl Default for LinkParams {
    fn default() -> Self {
        LinkParams { runahead_us: 1000, recv_timeout_ms: 8, target_fps_x10: 600, auto: true }
    }
}

fn write_link_params(out: &mut Vec<u8>, link: &LinkParams) {
    out.extend_from_slice(&link.runahead_us.to_le_bytes());
    out.extend_from_slice(&link.recv_timeout_ms.to_le_bytes());
    out.extend_from_slice(&link.target_fps_x10.to_le_bytes());
    out.push(u8::from(link.auto));
}

fn read_link_params(bytes: &[u8]) -> io::Result<LinkParams> {
    if bytes.len() < 9 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated LinkParams"));
    }
    Ok(LinkParams {
        runahead_us: u32::from_le_bytes(bytes[0..4].try_into().unwrap_or_default()),
        recv_timeout_ms: u16::from_le_bytes(bytes[4..6].try_into().unwrap_or_default()),
        target_fps_x10: u16::from_le_bytes(bytes[6..8].try_into().unwrap_or_default()),
        auto: bytes[8] != 0,
    })
}

impl ControlMessage {
    fn msg_type(&self) -> u8 {
        match self {
            ControlMessage::Hello { .. } => 0x01,
            ControlMessage::Welcome { .. } => 0x02,
            ControlMessage::Reject { .. } => 0x03,
            ControlMessage::PlayerList { .. } => 0x04,
            ControlMessage::RomChanged { .. } => 0x05,
            ControlMessage::MpReady { .. } => 0x06,
            ControlMessage::Heartbeat { .. } => 0x07,
            ControlMessage::LinkParams { .. } => 0x08,
            ControlMessage::Leave => 0x09,
        }
    }

    /// Encodes this message with its `4E4C4E4Ch` header, ready to be
    /// length-prefixed and written to the control socket.
    pub fn encode(&self, sender_id: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&CONTROL_MAGIC.to_le_bytes());
        out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        out.push(self.msg_type());
        out.push(sender_id);

        match self {
            ControlMessage::Hello { player_name, rom_fingerprint, mac_suffix, udp_port } => {
                write_fixed(&mut out, player_name, NAME_LEN);
                out.extend_from_slice(rom_fingerprint);
                out.extend_from_slice(mac_suffix);
                out.extend_from_slice(&udp_port.to_le_bytes());
            }
            ControlMessage::Welcome {
                player_id,
                max_players,
                room_name,
                host_rom_fingerprint,
                host_mp_port,
                link,
            } => {
                out.push(*player_id);
                out.push(*max_players);
                write_fixed(&mut out, room_name, NAME_LEN);
                out.extend_from_slice(host_rom_fingerprint);
                out.extend_from_slice(&host_mp_port.to_le_bytes());
                write_link_params(&mut out, link);
            }
            ControlMessage::Reject { reason } => out.push(reason.to_byte()),
            ControlMessage::PlayerList { players } => {
                out.push(players.len() as u8);
                for p in players {
                    out.push(p.id);
                    write_fixed(&mut out, &p.name, NAME_LEN);
                    out.extend_from_slice(&p.rom_fingerprint);
                    let flags = u8::from(p.mp_ready) | (u8::from(p.is_host) << 1);
                    out.push(flags);
                    out.extend_from_slice(&p.rtt_ms.to_le_bytes());
                    out.extend_from_slice(&p.fps_x10.to_le_bytes());
                }
            }
            ControlMessage::RomChanged { rom_fingerprint } => {
                out.extend_from_slice(rom_fingerprint);
            }
            ControlMessage::MpReady { ready } => out.push(u8::from(*ready)),
            ControlMessage::Heartbeat {
                tick,
                sustainable_fps_x10,
                blocked_ms_avg,
                reply_success_x1000,
            } => {
                out.extend_from_slice(&tick.to_le_bytes());
                out.extend_from_slice(&sustainable_fps_x10.to_le_bytes());
                out.extend_from_slice(&blocked_ms_avg.to_le_bytes());
                out.extend_from_slice(&reply_success_x1000.to_le_bytes());
            }
            ControlMessage::LinkParams { link } => write_link_params(&mut out, link),
            ControlMessage::Leave => {}
        }
        out
    }

    /// Decodes a message body previously produced by [`ControlMessage::encode`].
    /// Returns the message and the sender id from the header.
    ///
    /// # Errors
    /// Returns an error if the magic/version don't match, the message type
    /// is unrecognized, or the payload is too short for its type.
    pub fn decode(bytes: &[u8]) -> io::Result<(u8, ControlMessage)> {
        if bytes.len() < 8 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "control message too short"));
        }
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or_default());
        if magic != CONTROL_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad control magic"));
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap_or_default());
        if version != PROTOCOL_VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "protocol version mismatch"));
        }
        let msg_type = bytes[6];
        let sender_id = bytes[7];
        let body = &bytes[8..];

        let msg = match msg_type {
            0x01 => {
                if body.len() < NAME_LEN + FINGERPRINT_LEN + 3 + 2 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated Hello"));
                }
                let mut fp = [0u8; FINGERPRINT_LEN];
                fp.copy_from_slice(&body[NAME_LEN..NAME_LEN + FINGERPRINT_LEN]);
                let mut mac = [0u8; 3];
                mac.copy_from_slice(
                    &body[NAME_LEN + FINGERPRINT_LEN..NAME_LEN + FINGERPRINT_LEN + 3],
                );
                let port_off = NAME_LEN + FINGERPRINT_LEN + 3;
                let udp_port =
                    u16::from_le_bytes(body[port_off..port_off + 2].try_into().unwrap_or_default());
                ControlMessage::Hello {
                    player_name: read_fixed(&body[..NAME_LEN]),
                    rom_fingerprint: fp,
                    mac_suffix: mac,
                    udp_port,
                }
            }
            0x02 => {
                if body.len() < 2 + NAME_LEN + FINGERPRINT_LEN + 2 + 9 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated Welcome"));
                }
                let mut fp = [0u8; FINGERPRINT_LEN];
                let fp_off = 2 + NAME_LEN;
                fp.copy_from_slice(&body[fp_off..fp_off + FINGERPRINT_LEN]);
                let port_off = fp_off + FINGERPRINT_LEN;
                let host_mp_port =
                    u16::from_le_bytes(body[port_off..port_off + 2].try_into().unwrap_or_default());
                ControlMessage::Welcome {
                    player_id: body[0],
                    max_players: body[1],
                    room_name: read_fixed(&body[2..2 + NAME_LEN]),
                    host_rom_fingerprint: fp,
                    host_mp_port,
                    link: read_link_params(&body[port_off + 2..])?,
                }
            }
            0x03 => {
                if body.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated Reject"));
                }
                ControlMessage::Reject { reason: RejectReason::from_byte(body[0]) }
            }
            0x04 => {
                if body.is_empty() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated PlayerList",
                    ));
                }
                let count = body[0] as usize;
                const REC_LEN: usize = 1 + NAME_LEN + FINGERPRINT_LEN + 1 + 2 + 2;
                let mut players = Vec::with_capacity(count);
                let mut off = 1;
                for _ in 0..count {
                    if body.len() < off + REC_LEN {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated PlayerList record",
                        ));
                    }
                    let id = body[off];
                    let name = read_fixed(&body[off + 1..off + 1 + NAME_LEN]);
                    let fp_off = off + 1 + NAME_LEN;
                    let mut fp = [0u8; FINGERPRINT_LEN];
                    fp.copy_from_slice(&body[fp_off..fp_off + FINGERPRINT_LEN]);
                    let flags = body[fp_off + FINGERPRINT_LEN];
                    let rtt_off = fp_off + FINGERPRINT_LEN + 1;
                    let rtt_ms = u16::from_le_bytes(
                        body[rtt_off..rtt_off + 2].try_into().unwrap_or_default(),
                    );
                    let fps_x10 = u16::from_le_bytes(
                        body[rtt_off + 2..rtt_off + 4].try_into().unwrap_or_default(),
                    );
                    players.push(PlayerRecord {
                        id,
                        name,
                        rom_fingerprint: fp,
                        mp_ready: flags & 1 != 0,
                        is_host: flags & 2 != 0,
                        rtt_ms,
                        fps_x10,
                    });
                    off += REC_LEN;
                }
                ControlMessage::PlayerList { players }
            }
            0x05 => {
                if body.len() < FINGERPRINT_LEN {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated RomChanged",
                    ));
                }
                let mut fp = [0u8; FINGERPRINT_LEN];
                fp.copy_from_slice(&body[..FINGERPRINT_LEN]);
                ControlMessage::RomChanged { rom_fingerprint: fp }
            }
            0x06 => {
                if body.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated MpReady"));
                }
                ControlMessage::MpReady { ready: body[0] != 0 }
            }
            0x07 => {
                if body.len() < 10 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated Heartbeat",
                    ));
                }
                ControlMessage::Heartbeat {
                    tick: u32::from_le_bytes(body[0..4].try_into().unwrap_or_default()),
                    sustainable_fps_x10: u16::from_le_bytes(
                        body[4..6].try_into().unwrap_or_default(),
                    ),
                    blocked_ms_avg: u16::from_le_bytes(body[6..8].try_into().unwrap_or_default()),
                    reply_success_x1000: u16::from_le_bytes(
                        body[8..10].try_into().unwrap_or_default(),
                    ),
                }
            }
            0x08 => ControlMessage::LinkParams { link: read_link_params(body)? },
            0x09 => ControlMessage::Leave,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown control message type",
                ));
            }
        };
        Ok((sender_id, msg))
    }
}

/// Writes one length-prefixed control message to `stream`.
///
/// # Errors
/// Propagates any I/O error from the underlying write.
pub fn write_framed(
    stream: &mut impl Write,
    msg: &ControlMessage,
    sender_id: u8,
) -> io::Result<()> {
    let payload = msg.encode(sender_id);
    stream.write_all(&(payload.len() as u32).to_le_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()
}

/// Blocks reading one length-prefixed control message from `stream`.
///
/// # Errors
/// Propagates I/O errors, including a clean EOF (`UnexpectedEof`) when the
/// peer closes the connection.
pub fn read_framed(stream: &mut impl Read) -> io::Result<(u8, ControlMessage)> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    // A length beyond any real message is almost certainly a corrupted
    // stream, not a legitimate huge payload; refuse to allocate for it.
    if len > 1024 * 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control message implausibly large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    ControlMessage::decode(&buf)
}

/// MP-relay frame category, mirroring `nds_core::nds::MpFrameKind`. Kept as
/// a separate wire-only enum (rather than depending on the exact
/// `nds_core` byte encoding) so the wire format is documented and
/// versioned independently of the in-process enum's representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFrameKind {
    Packet,
    Cmd,
    Reply,
    Ack,
}

impl WireFrameKind {
    const fn to_byte(self) -> u8 {
        match self {
            WireFrameKind::Packet => 0,
            WireFrameKind::Cmd => 1,
            WireFrameKind::Reply => 2,
            WireFrameKind::Ack => 3,
        }
    }

    const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(WireFrameKind::Packet),
            1 => Some(WireFrameKind::Cmd),
            2 => Some(WireFrameKind::Reply),
            3 => Some(WireFrameKind::Ack),
            _ => None,
        }
    }
}

/// One MP-relay UDP datagram: 28-byte header (`docs/design/design_lan.md`
/// §5.4) followed by the 12-byte hardware header + 802.11 frame.
#[derive(Debug, Clone)]
pub struct MpDatagram {
    pub sender_id: u8,
    pub kind: WireFrameKind,
    pub aid: u16,
    pub send_seq: u32,
    pub timestamp_us: u64,
    pub runahead_us: u32,
    pub payload: Vec<u8>,
}

pub const MP_HEADER_LEN: usize = 28;

impl MpDatagram {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MP_HEADER_LEN + self.payload.len());
        out.extend_from_slice(&MP_MAGIC.to_le_bytes());
        out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        out.push(self.sender_id);
        out.push(self.kind.to_byte());
        out.extend_from_slice(&self.aid.to_le_bytes());
        out.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.send_seq.to_le_bytes());
        out.extend_from_slice(&self.timestamp_us.to_le_bytes());
        out.extend_from_slice(&self.runahead_us.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// # Errors
    /// Returns an error if the datagram is too short, has the wrong magic,
    /// an unsupported protocol version, or an unrecognized frame kind.
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < MP_HEADER_LEN {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "MP datagram too short"));
        }
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or_default());
        if magic != MP_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad MP magic"));
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap_or_default());
        if version != PROTOCOL_VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "MP protocol version mismatch"));
        }
        let kind = WireFrameKind::from_byte(bytes[7])
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown MP frame kind"))?;
        let aid = u16::from_le_bytes(bytes[8..10].try_into().unwrap_or_default());
        let payload_len = u16::from_le_bytes(bytes[10..12].try_into().unwrap_or_default()) as usize;
        if bytes.len() < MP_HEADER_LEN + payload_len {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "MP payload truncated"));
        }
        Ok(MpDatagram {
            sender_id: bytes[6],
            kind,
            aid,
            send_seq: u32::from_le_bytes(bytes[12..16].try_into().unwrap_or_default()),
            timestamp_us: u64::from_le_bytes(bytes[16..24].try_into().unwrap_or_default()),
            runahead_us: u32::from_le_bytes(bytes[24..28].try_into().unwrap_or_default()),
            payload: bytes[MP_HEADER_LEN..MP_HEADER_LEN + payload_len].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips() {
        let msg = ControlMessage::Hello {
            player_name: "Luna".to_owned(),
            rom_fingerprint: [7u8; FINGERPRINT_LEN],
            mac_suffix: [1, 2, 3],
            udp_port: 7065,
        };
        let encoded = msg.encode(0);
        let (sender, decoded) = ControlMessage::decode(&encoded).expect("decode");
        assert_eq!(sender, 0);
        match decoded {
            ControlMessage::Hello { player_name, rom_fingerprint, mac_suffix, udp_port } => {
                assert_eq!(player_name, "Luna");
                assert_eq!(rom_fingerprint, [7u8; FINGERPRINT_LEN]);
                assert_eq!(mac_suffix, [1, 2, 3]);
                assert_eq!(udp_port, 7065);
            }
            other => panic!("wrong variant decoded: {other:?}"),
        }
    }

    #[test]
    fn player_list_round_trips() {
        let msg = ControlMessage::PlayerList {
            players: vec![
                PlayerRecord {
                    id: 0,
                    name: "Host".to_owned(),
                    rom_fingerprint: [1; FINGERPRINT_LEN],
                    is_host: true,
                    mp_ready: true,
                    rtt_ms: 0,
                    fps_x10: 600,
                },
                PlayerRecord {
                    id: 1,
                    name: "Guest".to_owned(),
                    rom_fingerprint: [2; FINGERPRINT_LEN],
                    is_host: false,
                    mp_ready: false,
                    rtt_ms: 12,
                    fps_x10: 598,
                },
            ],
        };
        let encoded = msg.encode(0);
        let (_, decoded) = ControlMessage::decode(&encoded).expect("decode");
        match decoded {
            ControlMessage::PlayerList { players } => {
                assert_eq!(players.len(), 2);
                assert_eq!(players[0].name, "Host");
                assert!(players[0].is_host);
                assert_eq!(players[1].rtt_ms, 12);
            }
            other => panic!("wrong variant decoded: {other:?}"),
        }
    }

    #[test]
    fn mp_datagram_round_trips() {
        let dgram = MpDatagram {
            sender_id: 1,
            kind: WireFrameKind::Cmd,
            aid: 3,
            send_seq: 42,
            timestamp_us: 123_456,
            runahead_us: 2000,
            payload: vec![1, 2, 3, 4, 5],
        };
        let encoded = dgram.encode();
        assert_eq!(encoded.len(), MP_HEADER_LEN + 5);
        let decoded = MpDatagram::decode(&encoded).expect("decode");
        assert_eq!(decoded.sender_id, 1);
        assert_eq!(decoded.kind, WireFrameKind::Cmd);
        assert_eq!(decoded.aid, 3);
        assert_eq!(decoded.send_seq, 42);
        assert_eq!(decoded.timestamp_us, 123_456);
        assert_eq!(decoded.runahead_us, 2000);
        assert_eq!(decoded.payload, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = ControlMessage::Leave.encode(0);
        bytes[0] = 0;
        assert!(ControlMessage::decode(&bytes).is_err());
    }
}
