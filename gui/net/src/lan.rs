//! LAN multiplayer: melonDS's `src/net/LAN.cpp` ported to Rust.
//!
//! This is the socket-owning half of the [`nds_core::net`] port. It lives
//! here rather than in `nds-core` because the core crate owns no sockets.
//! Structurally it follows the original exactly: UDP broadcast discovery
//! on port [`DISCOVERY_PORT`], an ENet host on port [`LAN_PORT`] with two
//! channels ([`Channel::Cmd`] for control, [`Channel::Mp`] for MP frames),
//! a 16-entry player table, and the same
//! `Process`/`ProcessLAN`/`SendPacketGeneric`/`RecvPacketGeneric` flow.
//!
//! # Relationship to [`crate::room`]
//! [`crate::room::Room`] is a *different*, lunaris-native LAN protocol
//! (TCP control channel, adaptive pacing, ROM-fingerprint gating). Both are
//! offered; they are not wire-compatible with each other, and only this one
//! is wire-shaped like melonDS. Pick between them with
//! [`crate::MpInterfaceSelector`].
//!
//! # Deliberate deviations from the C++
//! Each of these replaces something the original does with a raw pointer or
//! a C-layout assumption:
//!
//! * `ENetPeer::data` pointed straight into the `Players` array. Here a peer
//!   is mapped to a player index through [`Lan`]'s `peer_players` table.
//! * `ENetPacket::userData` stored the sending `ENetPeer*` so a reply could
//!   be unicast back to the host. Here the queued frame carries the
//!   sender's [`PeerID`].
//! * `memcpy(&cmd[9], &MyPlayer, sizeof(Player))` puts a padded C struct on
//!   the wire. [`Player::to_bytes`] defines an explicit
//!   [`PLAYER_ENCODED_LEN`]-byte encoding instead, so this port is
//!   self-consistent but *not* binary-compatible with melonDS itself.
//! * `header->Magic` was overwritten with the packet's arrival time and read
//!   back for the staleness test. That field keeps its meaning here; the
//!   arrival time is a separate field of [`RxPacket`].
//! * `packets[(aid - 1) * 1024]` is guarded against `aid == 0` (which would
//!   underflow) and `aid > 15`, exactly as in
//!   [`nds_core::net::local`].
//! * `enet_host_service` took a blocking timeout; `rusty_enet`'s
//!   [`Host::service`] is non-blocking, so [`Lan::service_for`] polls to a
//!   deadline instead.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nds_core::net::mp_interface::{
    DEFAULT_RECV_TIMEOUT, MAX_INSTANCES, MP_PACKET_MAGIC, MpFrameCategory, MpFrameType,
    MpInterface, MpPacketHeader, MpRecvResult,
};
use rusty_enet::{Event, Host, HostSettings, Packet, PeerID};

/// UDP port session beacons are broadcast on.
pub const DISCOVERY_PORT: u16 = 7063;
/// UDP port the ENet host binds to.
pub const LAN_PORT: u16 = 7064;

/// `'LAND'` — tags a discovery beacon.
const DISCOVERY_MAGIC: u32 = 0x444E_414C;
/// `'LANP'` — tags a control-channel handshake message.
const LAN_MAGIC: u32 = 0x504E_414C;
/// Bumped whenever the control protocol changes incompatibly.
const PROTOCOL_VERSION: u32 = 1;

/// How often a host re-broadcasts its beacon, and how long a client keeps a
/// silent host in its discovery list.
const DISCOVERY_INTERVAL_MS: u32 = 1000;
const DISCOVERY_EXPIRY_MS: u32 = 5000;

/// Wall-clock budget for the initial client handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(5000);

/// How long an inbound MP frame may sit in the receive queue before it is
/// assumed stale (one video frame's worth of milliseconds).
const RX_STALE_MS: u32 = 16;

/// Poll interval used to emulate ENet's blocking service call.
const SERVICE_POLL: Duration = Duration::from_micros(250);

/// ENet channels, matching melonDS's anonymous enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Channel {
    /// Control commands.
    Cmd = 0,
    /// MP frame exchange.
    Mp = 1,
}

/// Control-channel command bytes, matching melonDS's `Cmd_*` enum.
mod cmd {
    /// host -> client: initialise a new client and assign its ID.
    pub const CLIENT_INIT: u8 = 1;
    /// client -> host: the client's player record.
    pub const PLAYER_INFO: u8 = 2;
    /// host -> client: the updated player list.
    pub const PLAYER_LIST: u8 = 3;
    /// both: this player is now ready to exchange MP frames.
    pub const PLAYER_CONNECT: u8 = 4;
    /// both: this player has stopped exchanging MP frames.
    pub const PLAYER_DISCONNECT: u8 = 5;
}

/// State of one entry in the 16-slot player table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerStatus {
    /// No player occupies this slot.
    #[default]
    None,
    /// A connected game client.
    Client,
    /// The game host.
    Host,
    /// A player that is still completing the handshake.
    Connecting,
    /// A player that has dropped out.
    Disconnected,
}

impl PlayerStatus {
    const fn to_byte(self) -> u8 {
        match self {
            PlayerStatus::None => 0,
            PlayerStatus::Client => 1,
            PlayerStatus::Host => 2,
            PlayerStatus::Connecting => 3,
            PlayerStatus::Disconnected => 4,
        }
    }

    const fn from_byte(byte: u8) -> Self {
        match byte {
            1 => PlayerStatus::Client,
            2 => PlayerStatus::Host,
            3 => PlayerStatus::Connecting,
            4 => PlayerStatus::Disconnected,
            _ => PlayerStatus::None,
        }
    }
}

/// One entry of the LAN session's player table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    /// Slot index, `0..16`. The host is always 0.
    pub id: u8,
    /// Display name, at most 31 bytes (melonDS's `char Name[32]`).
    pub name: String,
    /// Current state of this slot.
    pub status: PlayerStatus,
    /// Address this player is reachable at.
    pub address: Ipv4Addr,
    /// `true` for the entry describing this process.
    pub is_local_player: bool,
    /// Last measured round-trip time in milliseconds.
    pub ping: u32,
}

impl Default for Player {
    fn default() -> Self {
        Player {
            id: 0,
            name: String::new(),
            status: PlayerStatus::None,
            address: Ipv4Addr::UNSPECIFIED,
            is_local_player: false,
            ping: 0,
        }
    }
}

/// Wire size of one [`Player`]: id, 32-byte NUL-padded name, status,
/// address, local flag, ping.
pub const PLAYER_ENCODED_LEN: usize = 1 + 32 + 1 + 4 + 1 + 4;

impl Player {
    /// Serializes to the fixed-size wire form.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PLAYER_ENCODED_LEN] {
        let mut out = [0u8; PLAYER_ENCODED_LEN];
        out[0] = self.id;
        // Truncate to 31 bytes so a NUL terminator always fits, matching
        // melonDS's `strncpy(player->Name, playername, 31)`.
        let name = self.name.as_bytes();
        let len = name.len().min(31);
        out[1..1 + len].copy_from_slice(&name[..len]);
        out[33] = self.status.to_byte();
        out[34..38].copy_from_slice(&self.address.octets());
        out[38] = u8::from(self.is_local_player);
        out[39..43].copy_from_slice(&self.ping.to_le_bytes());
        out
    }

    /// Deserializes from the fixed-size wire form. Invalid UTF-8 in the
    /// name is replaced rather than rejected, since the name is only ever
    /// displayed.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; PLAYER_ENCODED_LEN]) -> Self {
        let name_end = bytes[1..33].iter().position(|&b| b == 0).unwrap_or(32);
        Player {
            id: bytes[0],
            name: String::from_utf8_lossy(&bytes[1..1 + name_end]).into_owned(),
            status: PlayerStatus::from_byte(bytes[33]),
            address: Ipv4Addr::new(bytes[34], bytes[35], bytes[36], bytes[37]),
            is_local_player: bytes[38] != 0,
            ping: u32::from_le_bytes([bytes[39], bytes[40], bytes[41], bytes[42]]),
        }
    }
}

/// A session beacon as broadcast by a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryData {
    /// Always [`DISCOVERY_MAGIC`] on the wire.
    pub magic: u32,
    /// Always [`PROTOCOL_VERSION`] on the wire.
    pub version: u32,
    /// Host's monotonic millisecond counter when the beacon was sent, used
    /// to discard beacons that arrive out of order.
    pub tick: u32,
    /// Human-readable session name, at most 63 bytes.
    pub session_name: String,
    /// Players currently in the session.
    pub num_players: u8,
    /// Players the session has room for.
    pub max_players: u8,
    /// `0` = idle, `1` = playing.
    pub status: u8,
}

/// Wire size of one [`DiscoveryData`].
pub const DISCOVERY_ENCODED_LEN: usize = 4 + 4 + 4 + 64 + 1 + 1 + 1;

impl DiscoveryData {
    fn to_bytes(&self) -> [u8; DISCOVERY_ENCODED_LEN] {
        let mut out = [0u8; DISCOVERY_ENCODED_LEN];
        out[0..4].copy_from_slice(&self.magic.to_le_bytes());
        out[4..8].copy_from_slice(&self.version.to_le_bytes());
        out[8..12].copy_from_slice(&self.tick.to_le_bytes());
        let name = self.session_name.as_bytes();
        let len = name.len().min(63);
        out[12..12 + len].copy_from_slice(&name[..len]);
        out[76] = self.num_players;
        out[77] = self.max_players;
        out[78] = self.status;
        out
    }

    fn from_bytes(bytes: &[u8; DISCOVERY_ENCODED_LEN]) -> Self {
        let word = |lo: usize| -> u32 {
            u32::from_le_bytes([bytes[lo], bytes[lo + 1], bytes[lo + 2], bytes[lo + 3]])
        };
        let name_end = bytes[12..76].iter().position(|&b| b == 0).unwrap_or(64);
        DiscoveryData {
            magic: word(0),
            version: word(4),
            tick: word(8),
            session_name: String::from_utf8_lossy(&bytes[12..12 + name_end]).into_owned(),
            num_players: bytes[76],
            max_players: bytes[77],
            status: bytes[78],
        }
    }
}

/// A discovered session plus the local time it was last heard from.
///
/// melonDS reuses `DiscoveryData::Magic` to store the arrival tick once the
/// beacon has been validated; keeping it in its own field preserves that
/// logic without giving `magic` two meanings.
#[derive(Debug, Clone)]
pub struct DiscoveryEntry {
    /// The beacon as received.
    pub data: DiscoveryData,
    /// Local millisecond counter when it arrived.
    pub last_seen_ms: u32,
}

/// One MP frame waiting to be handed to the emulator.
struct RxPacket {
    /// Local millisecond counter when this frame arrived; melonDS stores
    /// this over the header's magic word.
    arrival_ms: u32,
    header: MpPacketHeader,
    payload: Vec<u8>,
    /// Peer that sent it, so a reply can be unicast back to the host.
    peer: PeerID,
}

/// Wrapper carrying the `Send` promise for the ENet host.
///
/// `rusty_enet::Host` holds raw pointers into its own allocation and so is
/// not automatically `Send`. It has no thread affinity — ENet only requires
/// exclusive access — and every path that touches this field goes through
/// [`LanSession`]'s mutex, so moving it between threads is sound. This is
/// the single unsafe item in the port; the protocol logic itself uses no
/// raw pointers.
struct SendHost(Host<UdpSocket>);

// SAFETY: the wrapped `Host` is owned exclusively by the `Lan` that holds
// it, which is itself only reachable through `LanSession`'s `Mutex`. No
// `Host` internals are shared with any other thread, and ENet imposes no
// thread affinity beyond exclusive access.
unsafe impl Send for SendHost {}

/// A LAN multiplayer session.
///
/// Port of melonDS's `LAN` class. Not usable directly as an
/// [`MpInterface`] — wrap it in a [`LanSession`], which is what makes it
/// shareable between the UI and the emulator.
pub struct Lan {
    active: bool,
    is_host: bool,
    host: Option<SendHost>,
    /// Peer for each occupied player slot.
    remote_peers: [Option<PeerID>; MAX_INSTANCES],
    /// Replaces melonDS's `ENetPeer::data` pointer into `Players`.
    peer_players: HashMap<PeerID, u8>,

    discovery_socket: Option<UdpSocket>,
    discovery_last_tick: u32,
    discovery_list: BTreeMap<Ipv4Addr, DiscoveryEntry>,

    players: [Player; MAX_INSTANCES],
    num_players: u8,
    max_players: u8,
    my_player: Player,
    host_address: Ipv4Addr,

    connected_bitmask: u16,

    mp_recv_timeout: Duration,
    last_host_id: Option<u8>,
    last_host_peer: Option<PeerID>,
    rx_queue: VecDeque<RxPacket>,

    frame_count: u32,
    /// Origin for [`Lan::ms_count`], standing in for
    /// `Platform::GetMSCount`.
    start: Instant,
}

impl Default for Lan {
    fn default() -> Self {
        Lan::new()
    }
}

impl Lan {
    /// Creates an inactive session. No socket is opened until
    /// [`Lan::start_host`] or [`Lan::start_client`] is called.
    ///
    /// melonDS calls `enet_initialize()` here; `rusty_enet` needs no global
    /// initialisation, so there is no fallible construction step.
    #[must_use]
    pub fn new() -> Self {
        Lan {
            active: false,
            is_host: false,
            host: None,
            remote_peers: [None; MAX_INSTANCES],
            peer_players: HashMap::new(),
            discovery_socket: None,
            discovery_last_tick: 0,
            discovery_list: BTreeMap::new(),
            players: std::array::from_fn(|_| Player::default()),
            num_players: 0,
            max_players: 0,
            my_player: Player::default(),
            host_address: Ipv4Addr::LOCALHOST,
            connected_bitmask: 0,
            mp_recv_timeout: DEFAULT_RECV_TIMEOUT,
            last_host_id: None,
            last_host_peer: None,
            rx_queue: VecDeque::new(),
            frame_count: 0,
            start: Instant::now(),
        }
    }

    /// Monotonic millisecond counter, standing in for melonDS's
    /// `Platform::GetMSCount`. Wraps like the original's `u32`.
    fn ms_count(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    /// `true` once a session has been started and not yet ended.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// `true` when this process is the session host.
    #[must_use]
    pub const fn is_host(&self) -> bool {
        self.is_host
    }

    /// Players currently in the session.
    #[must_use]
    pub const fn num_players(&self) -> u8 {
        self.num_players
    }

    /// Players the session has room for.
    #[must_use]
    pub const fn max_players(&self) -> u8 {
        self.max_players
    }

    /// Sessions heard from over the discovery socket, keyed by host
    /// address. Port of `LAN::GetDiscoveryList`.
    #[must_use]
    pub fn discovery_list(&self) -> BTreeMap<Ipv4Addr, DiscoveryData> {
        self.discovery_list.iter().map(|(&k, v)| (k, v.data.clone())).collect()
    }

    /// The occupied entries of the player table, with the local player's
    /// address rewritten to loopback and the host's to the known host
    /// address. Port of `LAN::GetPlayerList`.
    #[must_use]
    pub fn player_list(&self) -> Vec<Player> {
        self.players
            .iter()
            .filter(|p| p.status != PlayerStatus::None)
            .map(|p| {
                let mut player = p.clone();
                if player.id == self.my_player.id {
                    player.is_local_player = true;
                    player.address = Ipv4Addr::LOCALHOST;
                } else {
                    player.is_local_player = false;
                    if player.status == PlayerStatus::Host {
                        player.address = self.host_address;
                    }
                }
                player
            })
            .collect()
    }

    // ---------------------------------------------------------------
    // Discovery
    // ---------------------------------------------------------------

    /// Opens the broadcast discovery socket. Port of
    /// `LAN::StartDiscovery`.
    ///
    /// # Errors
    /// Returns any error from binding [`DISCOVERY_PORT`] or enabling
    /// broadcast on the socket. A second instance on the same machine will
    /// fail here with `AddrInUse`; the caller may treat that as
    /// "discovery unavailable" rather than fatal.
    pub fn start_discovery(&mut self) -> io::Result<()> {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;

        self.discovery_last_tick = self.ms_count();
        self.discovery_list.clear();
        self.discovery_socket = Some(socket);
        self.active = true;
        Ok(())
    }

    /// Closes the discovery socket. Port of `LAN::EndDiscovery`.
    pub fn end_discovery(&mut self) {
        self.discovery_socket = None;
        if !self.is_host {
            self.active = false;
        }
    }

    /// One discovery tick: hosts broadcast a beacon, clients collect them
    /// and expire silent hosts. Port of `LAN::ProcessDiscovery`.
    fn process_discovery(&mut self) {
        if self.discovery_socket.is_none() {
            return;
        }

        let tick = self.ms_count();
        if tick.wrapping_sub(self.discovery_last_tick) < DISCOVERY_INTERVAL_MS {
            return;
        }
        self.discovery_last_tick = tick;

        if self.is_host {
            self.broadcast_beacon(tick);
        } else {
            self.collect_beacons(tick);
        }
    }

    fn broadcast_beacon(&mut self, tick: u32) {
        let beacon = DiscoveryData {
            magic: DISCOVERY_MAGIC,
            version: PROTOCOL_VERSION,
            tick,
            session_name: format!("{}'s game", self.my_player.name),
            num_players: self.num_players,
            max_players: self.max_players,
            status: 0,
        };
        let target = SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT);
        if let Some(socket) = &self.discovery_socket {
            let _ = socket.send_to(&beacon.to_bytes(), target);
        }
    }

    fn collect_beacons(&mut self, tick: u32) {
        let mut buf = [0u8; DISCOVERY_ENCODED_LEN];
        loop {
            let Some(socket) = &self.discovery_socket else { return };
            let Ok((len, from)) = socket.recv_from(&mut buf) else { break };
            if len < DISCOVERY_ENCODED_LEN {
                continue;
            }
            let beacon = DiscoveryData::from_bytes(&buf);
            if beacon.magic != DISCOVERY_MAGIC
                || beacon.version != PROTOCOL_VERSION
                || beacon.max_players as usize > MAX_INSTANCES
                || beacon.num_players > beacon.max_players
            {
                continue;
            }
            let IpAddr::V4(key) = from.ip() else { continue };

            // Ignore a beacon older than the newest one already stored for
            // this host, so out-of-order UDP cannot roll the entry back.
            if let Some(existing) = self.discovery_list.get(&key)
                && beacon.tick <= existing.data.tick
            {
                continue;
            }
            self.discovery_list.insert(key, DiscoveryEntry { data: beacon, last_seen_ms: tick });
        }

        // Drop hosts that have not been heard from recently.
        self.discovery_list
            .retain(|_, entry| tick.wrapping_sub(entry.last_seen_ms) < DISCOVERY_EXPIRY_MS);
    }

    // ---------------------------------------------------------------
    // Session lifecycle
    // ---------------------------------------------------------------

    /// Starts hosting a session for up to `num_players` players, and begins
    /// advertising it. Port of `LAN::StartHost`.
    ///
    /// # Errors
    /// Returns `InvalidInput` if `num_players` exceeds 16, or any error
    /// from binding [`LAN_PORT`]. Discovery failing to bind is *not* an
    /// error: hosting continues without advertising, so that a second
    /// instance on the same machine can still host.
    pub fn start_host(&mut self, player_name: &str, num_players: u8) -> io::Result<()> {
        if num_players as usize > MAX_INSTANCES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a LAN session supports at most 16 players",
            ));
        }

        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, LAN_PORT))?;
        let host = new_enet_host(socket)?;

        let mut player = Player {
            id: 0,
            name: truncated_name(player_name),
            status: PlayerStatus::Host,
            address: Ipv4Addr::LOCALHOST,
            is_local_player: true,
            ping: 0,
        };
        player.name = truncated_name(&player.name);
        self.players[0] = player.clone();
        self.num_players = 1;
        self.max_players = num_players;
        self.my_player = player;

        self.host = Some(SendHost(host));
        self.host_address = Ipv4Addr::LOCALHOST;
        self.last_host_id = None;
        self.last_host_peer = None;
        self.active = true;
        self.is_host = true;

        // melonDS treats a failed `StartDiscovery` as fatal to hosting;
        // here it only means the session is not advertised, which keeps
        // two hosts on one machine workable.
        let _ = self.start_discovery();
        Ok(())
    }

    /// Connects to a host and completes the `ClientInit`/`PlayerInfo`
    /// handshake. Port of `LAN::StartClient`.
    ///
    /// # Errors
    /// Returns any socket error, or `TimedOut` if the handshake does not
    /// complete within [`CONNECT_TIMEOUT`], or `ConnectionRefused` if the
    /// host rejects this client (session full, or protocol mismatch).
    pub fn start_client(&mut self, player_name: &str, host_addr: Ipv4Addr) -> io::Result<()> {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
        let mut host = new_enet_host(socket)?;

        let target = SocketAddr::from(SocketAddrV4::new(host_addr, LAN_PORT));
        let peer_id = host
            .connect(target, 2, 0)
            .map(|peer| peer.id())
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "no ENet peer slot available"))?;

        self.my_player = Player {
            id: 0,
            name: truncated_name(player_name),
            status: PlayerStatus::Connecting,
            address: Ipv4Addr::UNSPECIFIED,
            is_local_player: true,
            ping: 0,
        };

        match self.run_client_handshake(&mut host) {
            Ok(()) => {}
            Err(e) => {
                host.peer_mut(peer_id).reset();
                return Err(e);
            }
        }

        self.host = Some(SendHost(host));
        self.host_address = host_addr;
        self.last_host_id = None;
        self.last_host_peer = None;
        self.remote_peers[0] = Some(peer_id);
        self.peer_players.insert(peer_id, 0);
        self.active = true;
        self.is_host = false;
        Ok(())
    }

    /// The synchronous connect/`ClientInit`/`PlayerInfo` exchange from
    /// `LAN::StartClient`'s inner loop.
    fn run_client_handshake(&mut self, host: &mut Host<UdpSocket>) -> io::Result<()> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let mut connected = false;

        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for the host to accept this client",
                ));
            }
            let Some(event) = service_until(host, deadline)? else { continue };

            match event {
                Event::Connect { .. } if !connected => connected = true,
                Event::Disconnect { .. } => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionRefused,
                        "the host rejected or dropped this client",
                    ));
                }
                Event::Receive { peer, channel_id, packet } if connected => {
                    if channel_id != Channel::Cmd as u8 {
                        continue;
                    }
                    let data = packet.data();
                    // `Cmd_ClientInit`: magic, version, assigned id, max players.
                    if data.len() != 11 || data[0] != cmd::CLIENT_INIT {
                        continue;
                    }
                    if read_u32(&data[1..5]) != LAN_MAGIC
                        || read_u32(&data[5..9]) != PROTOCOL_VERSION
                        || data[10] as usize > MAX_INSTANCES
                    {
                        continue;
                    }

                    self.max_players = data[10];
                    self.my_player.id = data[9];

                    let mut reply = Vec::with_capacity(9 + PLAYER_ENCODED_LEN);
                    reply.push(cmd::PLAYER_INFO);
                    reply.extend_from_slice(&LAN_MAGIC.to_le_bytes());
                    reply.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
                    reply.extend_from_slice(&self.my_player.to_bytes());
                    let _ = peer.send(Channel::Cmd as u8, &Packet::reliable(reply.as_slice()));
                    host.flush();
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// Tears the session down, disconnecting every peer. Port of
    /// `LAN::EndSession`.
    pub fn end_session(&mut self) {
        if !self.active {
            return;
        }
        if self.is_host {
            self.end_discovery();
        }
        self.active = false;
        self.rx_queue.clear();

        if let Some(SendHost(host)) = self.host.as_mut() {
            for id in 0..MAX_INSTANCES {
                if id as u8 == self.my_player.id {
                    continue;
                }
                if let Some(peer) = self.remote_peers[id].take()
                    && let Some(peer) = host.get_peer_mut(peer)
                {
                    peer.disconnect(0);
                }
            }
            host.flush();
        }

        self.host = None;
        self.peer_players.clear();
        self.remote_peers = [None; MAX_INSTANCES];
        self.is_host = false;
        self.connected_bitmask = 0;
    }

    // ---------------------------------------------------------------
    // Control-channel events
    // ---------------------------------------------------------------

    /// Broadcasts the full player table to every client. Port of
    /// `LAN::HostUpdatePlayerList`.
    fn host_update_player_list(&mut self) {
        let mut msg = Vec::with_capacity(2 + MAX_INSTANCES * PLAYER_ENCODED_LEN);
        msg.push(cmd::PLAYER_LIST);
        msg.push(self.num_players);
        for player in &self.players {
            msg.extend_from_slice(&player.to_bytes());
        }
        if let Some(SendHost(host)) = self.host.as_mut() {
            host.broadcast(Channel::Cmd as u8, &Packet::reliable(msg.as_slice()));
        }
    }

    /// Dispatches one non-MP event to the host or client handler. Port of
    /// `LAN::ProcessEvent`.
    fn process_event(&mut self, event: OwnedEvent) {
        if self.is_host {
            self.process_host_event(event);
        } else {
            self.process_client_event(event);
        }
    }

    /// Port of `LAN::ProcessHostEvent`.
    fn process_host_event(&mut self, event: OwnedEvent) {
        match event {
            OwnedEvent::Connect { peer, address } => self.host_on_connect(peer, address),
            OwnedEvent::Disconnect { peer } => self.host_on_disconnect(peer),
            OwnedEvent::Receive { peer, channel_id, data, address } => {
                if channel_id == Channel::Cmd as u8 {
                    self.host_on_command(peer, &data, address);
                }
            }
        }
    }

    fn host_on_connect(&mut self, peer: PeerID, address: Ipv4Addr) {
        if self.num_players >= self.max_players || self.num_players as usize >= MAX_INSTANCES {
            // Session full.
            self.disconnect_peer(peer);
            return;
        }

        // First free slot, scanning only as far as the current player
        // count, exactly as melonDS does.
        let mut id = MAX_INSTANCES;
        for candidate in 0..MAX_INSTANCES {
            if candidate >= self.num_players as usize
                || self.players[candidate].status == PlayerStatus::None
            {
                id = candidate;
                break;
            }
        }
        if id >= MAX_INSTANCES {
            self.disconnect_peer(peer);
            return;
        }

        let mut msg = Vec::with_capacity(11);
        msg.push(cmd::CLIENT_INIT);
        msg.extend_from_slice(&LAN_MAGIC.to_le_bytes());
        msg.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        msg.push(id as u8);
        msg.push(self.max_players);
        self.send_to_peer(peer, Channel::Cmd, &Packet::reliable(msg.as_slice()));

        self.players[id].id = id as u8;
        self.players[id].status = PlayerStatus::Connecting;
        self.players[id].address = address;
        self.num_players += 1;

        self.peer_players.insert(peer, id as u8);
        self.remote_peers[id] = Some(peer);
    }

    fn host_on_disconnect(&mut self, peer: PeerID) {
        let Some(id) = self.peer_players.remove(&peer) else { return };
        let index = id as usize;
        if index >= MAX_INSTANCES {
            return;
        }

        self.connected_bitmask &= !(1 << index);
        self.remote_peers[index] = None;
        self.players[index].id = 0;
        self.players[index].status = PlayerStatus::None;
        self.num_players = self.num_players.saturating_sub(1);

        self.host_update_player_list();
    }

    fn host_on_command(&mut self, peer: PeerID, data: &[u8], address: Ipv4Addr) {
        let Some(&command) = data.first() else { return };
        match command {
            cmd::PLAYER_INFO => {
                if data.len() != 9 + PLAYER_ENCODED_LEN {
                    return;
                }
                if read_u32(&data[1..5]) != LAN_MAGIC || read_u32(&data[5..9]) != PROTOCOL_VERSION {
                    self.disconnect_peer(peer);
                    return;
                }
                let Ok(encoded) = <&[u8; PLAYER_ENCODED_LEN]>::try_from(&data[9..]) else {
                    return;
                };
                let mut player = Player::from_bytes(encoded);

                let Some(&slot) = self.peer_players.get(&peer) else { return };
                // A client claiming an ID other than the one it was
                // assigned is a protocol violation.
                if player.id != slot {
                    self.disconnect_peer(peer);
                    return;
                }

                player.status = PlayerStatus::Client;
                player.address = address;
                self.players[slot as usize] = player;

                self.host_update_player_list();
            }
            cmd::PLAYER_CONNECT => {
                if data.len() != 1 {
                    return;
                }
                if let Some(&id) = self.peer_players.get(&peer) {
                    self.connected_bitmask |= 1 << id;
                }
            }
            cmd::PLAYER_DISCONNECT => {
                if data.len() != 1 {
                    return;
                }
                if let Some(&id) = self.peer_players.get(&peer) {
                    self.connected_bitmask &= !(1 << id);
                }
            }
            _ => {}
        }
    }

    /// Port of `LAN::ProcessClientEvent`.
    fn process_client_event(&mut self, event: OwnedEvent) {
        match event {
            OwnedEvent::Connect { peer, address } => self.client_on_connect(peer, address),
            OwnedEvent::Disconnect { peer } => self.client_on_disconnect(peer),
            OwnedEvent::Receive { peer, channel_id, data, .. } => {
                if channel_id == Channel::Cmd as u8 {
                    self.client_on_command(peer, &data);
                }
            }
        }
    }

    fn client_on_connect(&mut self, peer: PeerID, address: Ipv4Addr) {
        // Another client is opening a direct connection to us; accept it
        // only if we already know about a player at that address.
        let player_id = self.players.iter().enumerate().find_map(|(i, player)| {
            (i as u8 != self.my_player.id
                && player.status == PlayerStatus::Client
                && player.address == address)
                .then_some(i)
        });

        match player_id {
            Some(id) => {
                self.remote_peers[id] = Some(peer);
                self.peer_players.insert(peer, id as u8);
            }
            None => self.disconnect_peer(peer),
        }
    }

    fn client_on_disconnect(&mut self, peer: PeerID) {
        let Some(id) = self.peer_players.remove(&peer) else { return };
        let index = id as usize;
        if index >= MAX_INSTANCES {
            return;
        }
        self.connected_bitmask &= !(1 << index);
        self.remote_peers[index] = None;
        self.players[index].status = PlayerStatus::Disconnected;
    }

    fn client_on_command(&mut self, peer: PeerID, data: &[u8]) {
        let Some(&command) = data.first() else { return };
        match command {
            cmd::PLAYER_LIST => {
                let expected = 2 + MAX_INSTANCES * PLAYER_ENCODED_LEN;
                if data.len() != expected || data[1] as usize > MAX_INSTANCES {
                    return;
                }
                self.num_players = data[1];
                for i in 0..MAX_INSTANCES {
                    let lo = 2 + i * PLAYER_ENCODED_LEN;
                    let Ok(encoded) =
                        <&[u8; PLAYER_ENCODED_LEN]>::try_from(&data[lo..lo + PLAYER_ENCODED_LEN])
                    else {
                        return;
                    };
                    self.players[i] = Player::from_bytes(encoded);
                }
                self.connect_to_new_clients();
            }
            cmd::PLAYER_CONNECT => {
                if data.len() != 1 {
                    return;
                }
                if let Some(&id) = self.peer_players.get(&peer) {
                    self.connected_bitmask |= 1 << id;
                }
            }
            cmd::PLAYER_DISCONNECT => {
                if data.len() != 1 {
                    return;
                }
                if let Some(&id) = self.peer_players.get(&peer) {
                    self.connected_bitmask &= !(1 << id);
                }
            }
            _ => {}
        }
    }

    /// Opens direct connections to any client in the freshly received
    /// player list that we are not already peered with.
    fn connect_to_new_clients(&mut self) {
        let my_id = self.my_player.id;
        let targets: Vec<Ipv4Addr> = (0..MAX_INSTANCES)
            .filter(|&i| {
                i as u8 != my_id
                    && self.players[i].status == PlayerStatus::Client
                    && self.remote_peers[i].is_none()
            })
            .map(|i| self.players[i].address)
            .collect();

        let Some(SendHost(host)) = self.host.as_mut() else { return };
        for address in targets {
            let target = SocketAddr::from(SocketAddrV4::new(address, LAN_PORT));
            // A failure here just means the mesh link is missing; the
            // session continues through the host.
            let _ = host.connect(target, 2, 0);
        }
    }

    fn disconnect_peer(&mut self, peer: PeerID) {
        if let Some(SendHost(host)) = self.host.as_mut()
            && let Some(peer) = host.get_peer_mut(peer)
        {
            peer.disconnect(0);
        }
    }

    fn send_to_peer(&mut self, peer: PeerID, channel: Channel, packet: &Packet) {
        if let Some(SendHost(host)) = self.host.as_mut()
            && let Some(peer) = host.get_peer_mut(peer)
        {
            let _ = peer.send(channel as u8, packet);
        }
    }

    // ---------------------------------------------------------------
    // The service loop
    // ---------------------------------------------------------------

    /// Pumps ENet, routing MP frames to the receive queue and everything
    /// else to the control-channel handlers.
    ///
    /// Port of `LAN::ProcessLAN`, whose `type` parameter selects the
    /// caller's intent; see [`ProcessMode`].
    fn process_lan(&mut self, mode: ProcessMode) {
        if self.host.is_none() {
            return;
        }

        let mut time_last = self.ms_count();

        // Drop frames that have been sitting in the queue for longer than
        // a video frame; anything the core wants, it consumes promptly.
        while let Some(front) = self.rx_queue.front() {
            let packet_time = front.arrival_ms;
            if packet_time > time_last || packet_time < time_last.wrapping_sub(RX_STALE_MS) {
                self.rx_queue.pop_front();
                continue;
            }
            // A usable frame is at the head of the queue.
            match mode {
                ProcessMode::WaitForMpFrame => return,
                ProcessMode::PollMiscFrame => {
                    // Looking for a non-MP frame, so an MP frame here is
                    // not what the caller wants.
                    if front.header.frame_type.category() == Some(MpFrameCategory::Regular) {
                        return;
                    }
                    self.rx_queue.pop_front();
                }
                ProcessMode::Frame => {}
            }
            break;
        }

        let mut remaining =
            if mode == ProcessMode::WaitForMpFrame { self.mp_recv_timeout } else { Duration::ZERO };
        time_last = self.ms_count();

        loop {
            let Some(event) = self.service_for(remaining) else { break };

            match event {
                OwnedEvent::Receive { peer, channel_id, ref data, .. }
                    if channel_id == Channel::Mp as u8 =>
                {
                    if self.enqueue_mp_frame(peer, data) {
                        // Stop as soon as one MP frame is queued: draining
                        // further would consume frames the core has not
                        // asked for yet.
                        return;
                    }
                }
                other => self.process_event(other),
            }

            if mode == ProcessMode::WaitForMpFrame {
                let now = self.ms_count();
                if now < time_last {
                    return;
                }
                let elapsed = Duration::from_millis(u64::from(now - time_last));
                if elapsed >= remaining {
                    return;
                }
                remaining -= elapsed;
                time_last = now;
            }
        }
    }

    /// Validates an inbound MP frame and queues it. Returns `true` if it
    /// was queued.
    fn enqueue_mp_frame(&mut self, peer: PeerID, data: &[u8]) -> bool {
        if data.len() < MpPacketHeader::ENCODED_LEN {
            return false;
        }
        let Ok(head) =
            <&[u8; MpPacketHeader::ENCODED_LEN]>::try_from(&data[..MpPacketHeader::ENCODED_LEN])
        else {
            return false;
        };
        let header = MpPacketHeader::from_bytes(head);
        if header.magic != MP_PACKET_MAGIC || header.sender_id == u32::from(self.my_player.id) {
            return false;
        }

        let body = &data[MpPacketHeader::ENCODED_LEN..];
        let len = (header.length as usize).min(body.len());
        self.rx_queue.push_back(RxPacket {
            arrival_ms: self.ms_count(),
            header,
            payload: body[..len].to_vec(),
            peer,
        });
        true
    }

    /// Waits up to `budget` for one ENet event.
    ///
    /// `rusty_enet::Host::service` never blocks, so a non-zero budget is
    /// emulated by polling to a deadline. A zero budget performs exactly
    /// one service call, matching `enet_host_service(host, &ev, 0)`.
    fn service_for(&mut self, budget: Duration) -> Option<OwnedEvent> {
        let Some(SendHost(host)) = self.host.as_mut() else { return None };
        if budget.is_zero() {
            return host.service().ok().flatten().map(OwnedEvent::from_event);
        }
        let deadline = Instant::now() + budget;
        service_until(host, deadline).ok().flatten().map(OwnedEvent::from_event)
    }

    // ---------------------------------------------------------------
    // MpInterface implementation details
    // ---------------------------------------------------------------

    /// Port of `LAN::SendPacketGeneric`.
    fn send_packet_generic(
        &mut self,
        frame_type: MpFrameType,
        packet: &[u8],
        timestamp: u64,
    ) -> usize {
        if self.host.is_none() {
            return 0;
        }
        let len = packet.len();

        let header = MpPacketHeader {
            magic: MP_PACKET_MAGIC,
            sender_id: u32::from(self.my_player.id),
            frame_type,
            length: len as u32,
            timestamp,
        };
        let mut bytes = Vec::with_capacity(MpPacketHeader::ENCODED_LEN + len);
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(packet);

        // melonDS sends MP frames unsequenced-unreliable: a late frame is
        // worse than a lost one.
        let enet_packet = Packet::unreliable_unsequenced(bytes.as_slice());
        let unicast_target = (frame_type.category() == Some(MpFrameCategory::Reply))
            .then_some(self.last_host_peer)
            .flatten();

        match unicast_target {
            Some(peer) => self.send_to_peer(peer, Channel::Mp, &enet_packet),
            None => {
                if let Some(SendHost(host)) = self.host.as_mut() {
                    host.broadcast(Channel::Mp as u8, &enet_packet);
                }
            }
        }
        if let Some(SendHost(host)) = self.host.as_mut() {
            host.flush();
        }

        len
    }

    /// Port of `LAN::RecvPacketGeneric`.
    fn recv_packet_generic(&mut self, packet: &mut [u8], block: bool) -> MpRecvResult {
        if self.host.is_none() {
            return MpRecvResult::None;
        }
        self.process_lan(if block {
            ProcessMode::WaitForMpFrame
        } else {
            ProcessMode::PollMiscFrame
        });

        let Some(frame) = self.rx_queue.pop_front() else { return MpRecvResult::None };

        // melonDS clamps an inbound frame at 2 KiB before copying.
        let len = frame.payload.len().min(2048);
        if len != 0 && frame.header.frame_type.category() == Some(MpFrameCategory::Cmd) {
            self.last_host_id = u8::try_from(frame.header.sender_id).ok();
            self.last_host_peer = Some(frame.peer);
        }

        let copied = len.min(packet.len());
        packet[..copied].copy_from_slice(&frame.payload[..copied]);
        MpRecvResult::Frame {
            len: copied,
            frame_type: frame.header.frame_type,
            timestamp: frame.header.timestamp,
        }
    }
}

/// What a [`Lan::process_lan`] call is being made for. Port of melonDS's
/// `ProcessLAN(int type)` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessMode {
    /// `0` — the once-per-video-frame pump.
    Frame,
    /// `1` — a non-blocking check for a non-MP frame.
    PollMiscFrame,
    /// `2` — block (up to the receive timeout) for an MP frame.
    WaitForMpFrame,
}

/// An ENet event detached from the borrow on its host.
///
/// `rusty_enet::Event` holds `&mut Peer`, which would keep the host
/// mutably borrowed across the whole handler. Copying out the peer ID,
/// address and payload lets the handlers take `&mut self`.
enum OwnedEvent {
    Connect { peer: PeerID, address: Ipv4Addr },
    Disconnect { peer: PeerID },
    Receive { peer: PeerID, channel_id: u8, data: Vec<u8>, address: Ipv4Addr },
}

impl OwnedEvent {
    fn from_event(event: Event<'_, UdpSocket>) -> Self {
        match event {
            Event::Connect { peer, .. } => {
                OwnedEvent::Connect { peer: peer.id(), address: peer_ipv4(peer.address()) }
            }
            Event::Disconnect { peer, .. } => OwnedEvent::Disconnect { peer: peer.id() },
            Event::Receive { peer, channel_id, packet } => OwnedEvent::Receive {
                peer: peer.id(),
                channel_id,
                data: packet.data().to_vec(),
                address: peer_ipv4(peer.address()),
            },
        }
    }
}

fn peer_ipv4(address: Option<SocketAddr>) -> Ipv4Addr {
    match address {
        Some(SocketAddr::V4(addr)) => *addr.ip(),
        _ => Ipv4Addr::UNSPECIFIED,
    }
}

fn new_enet_host(socket: UdpSocket) -> io::Result<Host<UdpSocket>> {
    // 16 peers, 2 channels: melonDS's `enet_host_create(&addr, 16, 2, 0, 0)`.
    Host::new(socket, HostSettings { peer_limit: 16, channel_limit: 2, ..HostSettings::default() })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("ENet host setup failed: {e}")))
}

/// Polls `host` until it yields an event or `deadline` passes.
///
/// # Errors
/// Propagates any socket error reported by ENet.
fn service_until(
    host: &mut Host<UdpSocket>,
    deadline: Instant,
) -> io::Result<Option<Event<'_, UdpSocket>>> {
    // The loop must not hold a borrow of `host` across iterations, so it
    // tests for readiness first and only then services for real.
    loop {
        if host.service()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(SERVICE_POLL);
    }
    // An event was dispatched into the host's queue on the call above;
    // `check_events` hands it back without touching the socket again.
    Ok(host.check_events())
}

fn truncated_name(name: &str) -> String {
    // 31 bytes plus a NUL, mirroring `char Name[32]`. Truncating on a
    // character boundary keeps the result valid UTF-8.
    let mut end = name.len().min(31);
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// A shareable handle onto a [`Lan`] session.
///
/// The emulator holds one (inside an
/// [`nds_core::net::MpInterfaceTransport`]) and the UI holds another, so
/// [`LanSession::process`] can keep discovery and the player list alive
/// while the core is mid-frame. Cloning is cheap.
#[derive(Clone)]
pub struct LanSession {
    inner: Arc<Mutex<Lan>>,
}

impl Default for LanSession {
    fn default() -> Self {
        LanSession::new()
    }
}

impl LanSession {
    /// Creates an inactive session.
    #[must_use]
    pub fn new() -> Self {
        LanSession { inner: Arc::new(Mutex::new(Lan::new())) }
    }

    fn lock(&self) -> MutexGuard<'_, Lan> {
        // Recovering from a poisoned lock keeps a panic in the UI thread
        // from permanently killing an in-progress session; every field is
        // plain data that the protocol handlers re-validate anyway.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts hosting. See [`Lan::start_host`].
    ///
    /// # Errors
    /// Propagates [`Lan::start_host`]'s errors.
    pub fn start_host(&self, player_name: &str, num_players: u8) -> io::Result<()> {
        self.lock().start_host(player_name, num_players)
    }

    /// Joins a host. See [`Lan::start_client`].
    ///
    /// # Errors
    /// Propagates [`Lan::start_client`]'s errors.
    pub fn start_client(&self, player_name: &str, host_addr: Ipv4Addr) -> io::Result<()> {
        self.lock().start_client(player_name, host_addr)
    }

    /// Opens the discovery socket without joining anything, so the UI can
    /// list sessions on the network. See [`Lan::start_discovery`].
    ///
    /// # Errors
    /// Propagates [`Lan::start_discovery`]'s errors.
    pub fn start_discovery(&self) -> io::Result<()> {
        self.lock().start_discovery()
    }

    /// Closes the discovery socket.
    pub fn end_discovery(&self) {
        self.lock().end_discovery();
    }

    /// Tears the session down.
    pub fn end_session(&self) {
        self.lock().end_session();
    }

    /// `true` while a session is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.lock().is_active()
    }

    /// `true` when this process hosts the session.
    #[must_use]
    pub fn is_host(&self) -> bool {
        self.lock().is_host()
    }

    /// Current and maximum player counts.
    #[must_use]
    pub fn player_counts(&self) -> (u8, u8) {
        let lan = self.lock();
        (lan.num_players(), lan.max_players())
    }

    /// Snapshot of the player list, for UI display.
    #[must_use]
    pub fn player_list(&self) -> Vec<Player> {
        self.lock().player_list()
    }

    /// Snapshot of the discovered sessions, for UI display.
    #[must_use]
    pub fn discovery_list(&self) -> BTreeMap<Ipv4Addr, DiscoveryData> {
        self.lock().discovery_list()
    }
}

impl MpInterface for LanSession {
    /// Port of `LAN::Process`: the once-per-video-frame pump. **Must** be
    /// called from the frontend's frame loop, or discovery, the player
    /// list, and ping measurement all stall.
    fn process(&mut self) {
        let mut lan = self.lock();
        if !lan.active {
            return;
        }
        lan.process_discovery();
        lan.process_lan(ProcessMode::Frame);

        lan.frame_count += 1;
        if lan.frame_count < 60 {
            return;
        }
        lan.frame_count = 0;

        // Refresh round-trip times once a second.
        let my_id = lan.my_player.id;
        for i in 0..MAX_INSTANCES {
            if lan.players[i].status == PlayerStatus::None || i as u8 == my_id {
                continue;
            }
            let Some(peer_id) = lan.remote_peers[i] else { continue };
            let Some(SendHost(host)) = lan.host.as_mut() else { break };
            let Some(peer) = host.get_peer_mut(peer_id) else { continue };
            lan.players[i].ping = peer.round_trip_time().as_millis() as u32;
        }
    }

    /// Port of `LAN::Begin`: announces that this player is ready to
    /// exchange MP frames.
    fn begin(&mut self, _inst: u8) {
        let mut lan = self.lock();
        if lan.host.is_none() {
            return;
        }
        lan.connected_bitmask |= 1 << lan.my_player.id;
        lan.last_host_id = None;
        lan.last_host_peer = None;

        if let Some(SendHost(host)) = lan.host.as_mut() {
            host.broadcast(Channel::Cmd as u8, &Packet::reliable(&[cmd::PLAYER_CONNECT][..]));
        }
    }

    /// Port of `LAN::End`.
    fn end(&mut self, _inst: u8) {
        let mut lan = self.lock();
        if lan.host.is_none() {
            return;
        }
        let my_id = lan.my_player.id;
        lan.connected_bitmask &= !(1 << my_id);

        if let Some(SendHost(host)) = lan.host.as_mut() {
            host.broadcast(Channel::Cmd as u8, &Packet::reliable(&[cmd::PLAYER_DISCONNECT][..]));
        }
    }

    fn send_packet(&mut self, _inst: u8, data: &[u8], timestamp: u64) -> usize {
        self.lock().send_packet_generic(MpFrameType::REGULAR, data, timestamp)
    }

    fn recv_packet(&mut self, _inst: u8, data: &mut [u8]) -> MpRecvResult {
        self.lock().recv_packet_generic(data, false)
    }

    fn send_cmd(&mut self, _inst: u8, data: &[u8], timestamp: u64) -> usize {
        self.lock().send_packet_generic(MpFrameType::CMD, data, timestamp)
    }

    fn send_reply(&mut self, _inst: u8, data: &[u8], timestamp: u64, aid: u16) -> usize {
        self.lock().send_packet_generic(MpFrameType::reply(aid), data, timestamp)
    }

    fn send_ack(&mut self, _inst: u8, data: &[u8], timestamp: u64) -> usize {
        self.lock().send_packet_generic(MpFrameType::ACK, data, timestamp)
    }

    /// Port of `LAN::RecvHostPacket`.
    fn recv_host_packet(&mut self, _inst: u8, data: &mut [u8]) -> MpRecvResult {
        let mut lan = self.lock();
        if let Some(host_id) = lan.last_host_id
            && lan.connected_bitmask & (1 << host_id) == 0
        {
            return MpRecvResult::HostGone;
        }
        lan.recv_packet_generic(data, true)
    }

    /// Port of `LAN::RecvReplies`.
    fn recv_replies(&mut self, _inst: u8, data: &mut [u8], timestamp: u64, aid_mask: u16) -> u16 {
        let mut lan = self.lock();
        if lan.host.is_none() {
            return 0;
        }

        let mut answered = 0u16;
        let mut seen_mask = 1u16 << lan.my_player.id;
        let connected = lan.connected_bitmask;
        if seen_mask & connected == connected {
            return 0;
        }

        loop {
            lan.process_lan(ProcessMode::WaitForMpFrame);
            let Some(frame) = lan.rx_queue.pop_front() else {
                // No more replies available.
                return answered;
            };

            let is_reply = frame.header.frame_type.category() == Some(MpFrameCategory::Reply);
            // Wrapping, like melonDS: at timestamps below 32 the test is
            // vacuously true and every reply counts as stale.
            let stale = frame.header.timestamp < timestamp.wrapping_sub(32);
            if !is_reply || stale {
                continue;
            }

            let len = frame.payload.len().min(1024);
            if len != 0 {
                // Guarded against melonDS's unchecked `(aid - 1) * 1024`.
                let aid = frame.header.frame_type.aid();
                if (1..MAX_INSTANCES as u16).contains(&aid) {
                    let slot = (aid as usize - 1) * 1024;
                    let end = (slot + len).min(data.len());
                    if end > slot {
                        data[slot..end].copy_from_slice(&frame.payload[..end - slot]);
                    }
                    answered |= 1 << aid;
                }
            }

            seen_mask |= 1 << (frame.header.sender_id & 0xF);
            if seen_mask & connected == connected || answered & aid_mask == aid_mask {
                // Every client has replied.
                return answered;
            }
        }
    }

    fn recv_timeout(&self) -> Duration {
        self.lock().mp_recv_timeout
    }

    fn set_recv_timeout(&mut self, timeout: Duration) {
        self.lock().mp_recv_timeout = timeout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_round_trips_through_its_wire_form() {
        let player = Player {
            id: 5,
            name: "test player".to_owned(),
            status: PlayerStatus::Client,
            address: Ipv4Addr::new(192, 168, 1, 42),
            is_local_player: true,
            ping: 1234,
        };
        assert_eq!(Player::from_bytes(&player.to_bytes()), player);
    }

    #[test]
    fn overlong_player_names_are_truncated_to_31_bytes() {
        let player = Player { name: "x".repeat(64), ..Player::default() };
        let decoded = Player::from_bytes(&player.to_bytes());
        assert_eq!(decoded.name.len(), 31);
    }

    #[test]
    fn discovery_beacon_round_trips() {
        let beacon = DiscoveryData {
            magic: DISCOVERY_MAGIC,
            version: PROTOCOL_VERSION,
            tick: 987_654,
            session_name: "someone's game".to_owned(),
            num_players: 2,
            max_players: 4,
            status: 1,
        };
        let bytes = beacon.to_bytes();
        assert_eq!(bytes.len(), DISCOVERY_ENCODED_LEN);
        assert_eq!(DiscoveryData::from_bytes(&bytes), beacon);
    }

    #[test]
    fn player_list_message_length_matches_the_client_side_check() {
        // The host builds `1 + 1 + 16 * PLAYER_ENCODED_LEN`; the client
        // rejects anything else. A mismatch here would silently break the
        // player list.
        let mut msg = vec![cmd::PLAYER_LIST, 1];
        for _ in 0..MAX_INSTANCES {
            msg.extend_from_slice(&Player::default().to_bytes());
        }
        assert_eq!(msg.len(), 2 + MAX_INSTANCES * PLAYER_ENCODED_LEN);
    }

    #[test]
    fn client_init_message_is_eleven_bytes() {
        // `StartClient` rejects any other length.
        let mut msg = vec![cmd::CLIENT_INIT];
        msg.extend_from_slice(&LAN_MAGIC.to_le_bytes());
        msg.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        msg.push(0);
        msg.push(4);
        assert_eq!(msg.len(), 11);
    }

    #[test]
    fn player_info_message_length_matches_the_host_side_check() {
        let mut msg = vec![cmd::PLAYER_INFO];
        msg.extend_from_slice(&LAN_MAGIC.to_le_bytes());
        msg.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        msg.extend_from_slice(&Player::default().to_bytes());
        assert_eq!(msg.len(), 9 + PLAYER_ENCODED_LEN);
    }

    #[test]
    fn an_inactive_session_accepts_and_returns_nothing() {
        let mut lan = LanSession::new();
        assert!(!lan.is_active());
        assert_eq!(lan.send_packet(0, &[1, 2, 3], 0), 0);

        let mut buf = [0u8; 16];
        assert_eq!(lan.recv_packet(0, &mut buf), MpRecvResult::None);
        assert_eq!(lan.recv_replies(0, &mut buf, 0, 0xFFFF), 0);
        // `process` on an inactive session must be a no-op, not a panic.
        lan.process();
    }

    #[test]
    fn truncation_never_splits_a_utf8_character() {
        // 16 three-byte characters = 48 bytes; the 31-byte limit lands
        // mid-character.
        let name = "あ".repeat(16);
        let truncated = truncated_name(&name);
        assert!(truncated.len() <= 31);
        assert_eq!(truncated.chars().count(), 10);
    }
}
