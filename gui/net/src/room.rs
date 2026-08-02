//! Room lifecycle: hosting, joining by IP, player-list maintenance, and
//! wiring the pacing controller's output into the shared state
//! [`crate::transport::NetTransport`] reads. See
//! `docs/design/design_lan.md` §5.2 and §11.
//!
//! The room is deliberately independent of any loaded ROM: [`Room::host`]
//! and [`Room::join`] never touch `nds_core` beyond the [`MpTransport`]
//! boundary. Same-software gating (§10) is the caller's responsibility --
//! [`RoomHandle::set_rom_fingerprint`] just publishes a value other peers
//! compare, it doesn't enforce anything itself.

use std::{
    io,
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::Duration,
};

use nds_core::nds::LinkHints;

use crate::{
    pacing::{Controller, Measurements},
    transport::{NetTransport, PeerTable, SharedHints},
    wire::{self, ControlMessage, LinkParams, PlayerRecord, RejectReason},
};

/// Caller-supplied identity/room parameters, shared by both
/// [`Room::host`] and [`Room::join`].
#[derive(Debug, Clone)]
pub struct RoomConfig {
    pub player_name: String,
    pub room_name: String,
    pub rom_fingerprint: [u8; 16],
    pub mac_suffix: [u8; 3],
    pub max_players: u8,
    pub control_port: u16,
    pub mp_port: u16,
}

/// One entry of the room's live player list, for UI display
/// (`docs/design/design_lan.md` §11.3).
#[derive(Debug, Clone)]
pub struct PlayerView {
    pub id: u8,
    pub name: String,
    pub rom_fingerprint: [u8; 16],
    pub is_host: bool,
    pub mp_ready: bool,
    pub rtt_ms: u16,
    pub fps_x10: u16,
}

#[derive(Debug, Default)]
struct RoomState {
    players: Vec<PlayerView>,
    room_name: String,
    max_players: u8,
    link: LinkParams,
    last_error: Option<String>,
    self_id: u8,
    left: bool,
}

/// Cheap, `Clone`-able handle to a room's live state, safe to poll every
/// UI repaint. All the actual socket work happens on background threads
/// spawned by [`Room::host`]/[`Room::join`]; this handle only reads/writes
/// shared, mutex-protected state.
#[derive(Clone)]
pub struct RoomHandle {
    state: Arc<Mutex<RoomState>>,
    hints: Arc<SharedHints>,
    rom_fingerprint: Arc<Mutex<[u8; 16]>>,
    outbound: Sender<ControlMessage>,
    self_id: u8,
    is_host: bool,
}

impl RoomHandle {
    pub fn players(&self) -> Vec<PlayerView> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).players.clone()
    }

    pub fn room_name(&self) -> String {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).room_name.clone()
    }

    pub fn link_params(&self) -> LinkParams {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).link
    }

    pub fn link_hints(&self) -> LinkHints {
        self.hints.get()
    }

    pub fn last_error(&self) -> Option<String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).last_error.clone()
    }

    pub fn self_id(&self) -> u8 {
        self.self_id
    }

    pub fn is_host(&self) -> bool {
        self.is_host
    }

    /// `true` once the room (host or a peer) has torn itself down and this
    /// handle should be dropped by the UI.
    pub fn has_left(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).left
    }

    /// Publishes a new ROM fingerprint, e.g. after the local ROM changes
    /// (`docs/design/design_lan.md` §10.3). Broadcasts to the room so
    /// same-software matching stays current.
    pub fn set_rom_fingerprint(&self, fingerprint: [u8; 16]) {
        *self.rom_fingerprint.lock().unwrap_or_else(|e| e.into_inner()) = fingerprint;
        let _ = self.outbound.send(ControlMessage::RomChanged { rom_fingerprint: fingerprint });
    }

    pub fn set_mp_ready(&self, ready: bool) {
        let _ = self.outbound.send(ControlMessage::MpReady { ready });
    }

    /// Host-only: overrides the adaptive link controller
    /// (`docs/design/design_lan.md` §9.4). No-op for guests -- only the
    /// host may change link parameters.
    pub fn set_link_params(&self, link: LinkParams) {
        if self.is_host {
            let _ = self.outbound.send(ControlMessage::LinkParams { link });
        }
    }

    pub fn leave(&self) {
        let _ = self.outbound.send(ControlMessage::Leave);
        self.state.lock().unwrap_or_else(|e| e.into_inner()).left = true;
    }
}

/// A live room membership: the control-plane [`RoomHandle`] plus the
/// [`NetTransport`] to install via `NDS::set_mp_transport`.
pub struct Room {
    pub handle: RoomHandle,
    pub transport: NetTransport,
}

impl Room {
    /// Starts hosting a room: binds the control (TCP) and MP-relay (UDP)
    /// ports and becomes player id 0.
    ///
    /// # Errors
    /// Returns any error from binding either socket.
    pub fn host(cfg: &RoomConfig) -> io::Result<Room> {
        let listener = TcpListener::bind(("0.0.0.0", cfg.control_port))?;
        let udp_socket = UdpSocket::bind(("0.0.0.0", cfg.mp_port))?;

        let peers = Arc::new(PeerTable::default());
        peers.set(vec![(0, SocketAddr::new(IpAddr::from([127, 0, 0, 1]), cfg.mp_port))]);
        let hints_shared = Arc::new(SharedHints::default());
        hints_shared.set(Controller::new().hints());

        let transport = NetTransport::from_socket(
            udp_socket,
            0,
            0,
            Arc::clone(&peers),
            Arc::clone(&hints_shared),
        )?;

        let state = Arc::new(Mutex::new(RoomState {
            players: vec![PlayerView {
                id: 0,
                name: cfg.player_name.clone(),
                rom_fingerprint: cfg.rom_fingerprint,
                is_host: true,
                mp_ready: false,
                rtt_ms: 0,
                fps_x10: 0,
            }],
            room_name: cfg.room_name.clone(),
            max_players: cfg.max_players,
            link: LinkParams::default(),
            last_error: None,
            self_id: 0,
            left: false,
        }));
        let rom_fingerprint = Arc::new(Mutex::new(cfg.rom_fingerprint));

        let connections: Arc<Mutex<Vec<(u8, TcpStream)>>> = Arc::new(Mutex::new(Vec::new()));
        let (outbound_tx, outbound_rx) = std::sync::mpsc::channel::<ControlMessage>();
        let shutdown = Arc::new(AtomicBool::new(false));

        spawn_host_accept_loop(
            listener,
            Arc::clone(&state),
            Arc::clone(&peers),
            Arc::clone(&connections),
            cfg.mp_port,
            Arc::clone(&shutdown),
        );
        spawn_host_pacing_loop(
            Arc::clone(&state),
            Arc::clone(&hints_shared),
            Arc::clone(&connections),
            Arc::clone(&shutdown),
        );
        spawn_local_outbound_loop(outbound_rx, Arc::clone(&state), Arc::clone(&connections));

        Ok(Room {
            handle: RoomHandle {
                state,
                hints: hints_shared,
                rom_fingerprint,
                outbound: outbound_tx,
                self_id: 0,
                is_host: true,
            },
            transport,
        })
    }

    /// Joins a room hosted at `host_ip` by IP address
    /// (`docs/design/design_lan.md` §5.1/§11.3).
    ///
    /// # Errors
    /// Returns any error connecting to the host, binding the local UDP
    /// relay socket, or an early I/O failure while exchanging `Hello`/
    /// `Welcome`.
    pub fn join(cfg: &RoomConfig, host_ip: IpAddr) -> io::Result<Room> {
        let mut stream = TcpStream::connect((host_ip, cfg.control_port))?;
        let udp_socket = UdpSocket::bind(("0.0.0.0", 0))?;
        let local_udp_port = udp_socket.local_addr()?.port();

        wire::write_framed(
            &mut stream,
            &ControlMessage::Hello {
                player_name: cfg.player_name.clone(),
                rom_fingerprint: cfg.rom_fingerprint,
                mac_suffix: cfg.mac_suffix,
                udp_port: local_udp_port,
            },
            0xFF,
        )?;

        let (_sender, welcome) = wire::read_framed(&mut stream)?;
        let (player_id, room_name, max_players, host_fingerprint, host_mp_port, link) =
            match welcome {
                ControlMessage::Welcome {
                    player_id,
                    max_players,
                    room_name,
                    host_rom_fingerprint,
                    host_mp_port,
                    link,
                } => (player_id, room_name, max_players, host_rom_fingerprint, host_mp_port, link),
                ControlMessage::Reject { reason } => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionRefused,
                        reject_message(reason),
                    ));
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected reply to Hello",
                    ));
                }
            };

        let peers = Arc::new(PeerTable::default());
        peers.set(vec![(0, SocketAddr::new(host_ip, host_mp_port))]);
        let hints_shared = Arc::new(SharedHints::default());
        hints_shared.set(link.to_hints());

        let transport = NetTransport::from_socket(
            udp_socket,
            player_id,
            0,
            Arc::clone(&peers),
            Arc::clone(&hints_shared),
        )?;

        let state = Arc::new(Mutex::new(RoomState {
            players: vec![
                PlayerView {
                    id: 0,
                    name: String::new(),
                    rom_fingerprint: host_fingerprint,
                    is_host: true,
                    mp_ready: false,
                    rtt_ms: 0,
                    fps_x10: 0,
                },
                PlayerView {
                    id: player_id,
                    name: cfg.player_name.clone(),
                    rom_fingerprint: cfg.rom_fingerprint,
                    is_host: false,
                    mp_ready: false,
                    rtt_ms: 0,
                    fps_x10: 0,
                },
            ],
            room_name,
            max_players,
            link,
            last_error: None,
            self_id: player_id,
            left: false,
        }));
        let rom_fingerprint = Arc::new(Mutex::new(cfg.rom_fingerprint));

        let write_half = stream.try_clone()?;
        let write_half = Arc::new(Mutex::new(write_half));
        let (outbound_tx, outbound_rx) = std::sync::mpsc::channel::<ControlMessage>();
        let shutdown = Arc::new(AtomicBool::new(false));

        spawn_client_reader_loop(
            stream,
            Arc::clone(&state),
            Arc::clone(&peers),
            Arc::clone(&hints_shared),
            host_ip,
            host_mp_port,
            Arc::clone(&shutdown),
        );
        spawn_client_outbound_loop(outbound_rx, write_half, player_id, Arc::clone(&shutdown));

        Ok(Room {
            handle: RoomHandle {
                state,
                hints: hints_shared,
                rom_fingerprint,
                outbound: outbound_tx,
                self_id: player_id,
                is_host: false,
            },
            transport,
        })
    }
}

fn reject_message(reason: RejectReason) -> String {
    match reason {
        RejectReason::RoomFull => "room is full".to_owned(),
        RejectReason::VersionMismatch => "protocol version mismatch".to_owned(),
        RejectReason::MacCollision => {
            "MAC address collision -- randomize your MAC and retry".to_owned()
        }
        RejectReason::Banned => "banned from this room".to_owned(),
    }
}

impl LinkParams {
    const fn to_hints(self) -> LinkHints {
        LinkHints {
            runahead_us: self.runahead_us,
            recv_timeout: Duration::from_millis(self.recv_timeout_ms as u64),
        }
    }
}

fn broadcast(connections: &Mutex<Vec<(u8, TcpStream)>>, msg: &ControlMessage, sender_id: u8) {
    let mut conns = connections.lock().unwrap_or_else(|e| e.into_inner());
    conns.retain_mut(|(_, stream)| wire::write_framed(stream, msg, sender_id).is_ok());
}

fn player_list_message(state: &Mutex<RoomState>) -> ControlMessage {
    let guard = state.lock().unwrap_or_else(|e| e.into_inner());
    ControlMessage::PlayerList {
        players: guard
            .players
            .iter()
            .map(|p| PlayerRecord {
                id: p.id,
                name: p.name.clone(),
                rom_fingerprint: p.rom_fingerprint,
                is_host: p.is_host,
                mp_ready: p.mp_ready,
                rtt_ms: p.rtt_ms,
                fps_x10: p.fps_x10,
            })
            .collect(),
    }
}

fn spawn_host_accept_loop(
    listener: TcpListener,
    state: Arc<Mutex<RoomState>>,
    peers: Arc<PeerTable>,
    connections: Arc<Mutex<Vec<(u8, TcpStream)>>>,
    mp_port: u16,
    shutdown: Arc<AtomicBool>,
) {
    // Per-connection UDP addresses, tracked alongside the TCP write-clone
    // table so `PeerTable` (consumed by `NetTransport`) can be rebuilt
    // whenever membership changes.
    let udp_addrs: Arc<Mutex<Vec<(u8, SocketAddr)>>> =
        Arc::new(Mutex::new(vec![(0, SocketAddr::new(IpAddr::from([127, 0, 0, 1]), mp_port))]));

    let _ = listener.set_nonblocking(true);
    std::thread::spawn(move || {
        loop {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            match listener.accept() {
                Ok((stream, addr)) => {
                    let _ = stream.set_nonblocking(false);
                    spawn_host_guest_thread(
                        stream,
                        addr,
                        Arc::clone(&state),
                        Arc::clone(&peers),
                        Arc::clone(&connections),
                        Arc::clone(&udp_addrs),
                        mp_port,
                        Arc::clone(&shutdown),
                    );
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return,
            }
        }
    });
}

#[expect(clippy::too_many_arguments, reason = "internal spawn helper, not part of the public API")]
fn spawn_host_guest_thread(
    mut stream: TcpStream,
    addr: SocketAddr,
    state: Arc<Mutex<RoomState>>,
    peers: Arc<PeerTable>,
    connections: Arc<Mutex<Vec<(u8, TcpStream)>>>,
    udp_addrs: Arc<Mutex<Vec<(u8, SocketAddr)>>>,
    mp_port: u16,
    shutdown: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let hello = match wire::read_framed(&mut stream) {
            Ok((_, msg)) => msg,
            Err(_) => return,
        };
        let ControlMessage::Hello { player_name, rom_fingerprint, udp_port, .. } = hello else {
            let _ = wire::write_framed(
                &mut stream,
                &ControlMessage::Reject { reason: RejectReason::VersionMismatch },
                0,
            );
            return;
        };

        let assigned_id = {
            let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.players.len() >= guard.max_players as usize {
                let _ = wire::write_framed(
                    &mut stream,
                    &ControlMessage::Reject { reason: RejectReason::RoomFull },
                    0,
                );
                return;
            }
            let used: Vec<u8> = guard.players.iter().map(|p| p.id).collect();
            let id = (1..guard.max_players).find(|i| !used.contains(i)).unwrap_or(1);
            guard.players.push(PlayerView {
                id,
                name: player_name,
                rom_fingerprint,
                is_host: false,
                mp_ready: false,
                rtt_ms: 0,
                fps_x10: 0,
            });
            id
        };

        {
            let mut addrs = udp_addrs.lock().unwrap_or_else(|e| e.into_inner());
            addrs.push((assigned_id, SocketAddr::new(addr.ip(), udp_port)));
            peers.set(addrs.clone());
        }

        let (room_name, max_players, host_fingerprint, link) = {
            let guard = state.lock().unwrap_or_else(|e| e.into_inner());
            let host_fp = guard.players.first().map(|p| p.rom_fingerprint).unwrap_or_default();
            (guard.room_name.clone(), guard.max_players, host_fp, guard.link)
        };
        if wire::write_framed(
            &mut stream,
            &ControlMessage::Welcome {
                player_id: assigned_id,
                max_players,
                room_name,
                host_rom_fingerprint: host_fingerprint,
                host_mp_port: mp_port,
                link,
            },
            0,
        )
        .is_err()
        {
            return;
        }

        if let Ok(write_clone) = stream.try_clone() {
            connections.lock().unwrap_or_else(|e| e.into_inner()).push((assigned_id, write_clone));
        }
        broadcast(&connections, &player_list_message(&state), 0);

        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            match wire::read_framed(&mut stream) {
                Ok((_, ControlMessage::RomChanged { rom_fingerprint })) => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(p) = guard.players.iter_mut().find(|p| p.id == assigned_id) {
                        p.rom_fingerprint = rom_fingerprint;
                    }
                    drop(guard);
                    broadcast(&connections, &player_list_message(&state), 0);
                }
                Ok((_, ControlMessage::MpReady { ready })) => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(p) = guard.players.iter_mut().find(|p| p.id == assigned_id) {
                        p.mp_ready = ready;
                    }
                    drop(guard);
                    broadcast(&connections, &player_list_message(&state), 0);
                }
                Ok((_, ControlMessage::Heartbeat { sustainable_fps_x10, .. })) => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(p) = guard.players.iter_mut().find(|p| p.id == assigned_id) {
                        p.fps_x10 = sustainable_fps_x10;
                    }
                }
                Ok((_, ControlMessage::Leave)) | Err(_) => break,
                Ok(_) => {}
            }
        }

        state.lock().unwrap_or_else(|e| e.into_inner()).players.retain(|p| p.id != assigned_id);
        connections.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _)| *id != assigned_id);
        udp_addrs.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _)| *id != assigned_id);
        peers.set(udp_addrs.lock().unwrap_or_else(|e| e.into_inner()).clone());
        broadcast(&connections, &player_list_message(&state), 0);
    });
}

fn spawn_host_pacing_loop(
    state: Arc<Mutex<RoomState>>,
    hints: Arc<SharedHints>,
    connections: Arc<Mutex<Vec<(u8, TcpStream)>>>,
    shutdown: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut controller = Controller::new();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            // Simplified per `docs/design/design_lan.md` §9.1: without a
            // deeper reply-success/jitter feed from the transport, use a
            // fixed "healthy link" measurement so the controller still
            // exercises its clamps/stability logic; a future pass can wire
            // `NetTransport`'s real reply/jitter counters through here.
            controller.evaluate(Measurements {
                reply_success: 1.0,
                jitter_us: 0,
                blocked_ms_avg: 0,
            });
            hints.set(controller.hints());

            let link = LinkParams {
                runahead_us: controller.runahead_us(),
                recv_timeout_ms: controller.recv_timeout_ms(),
                target_fps_x10: 600,
                auto: true,
            };
            state.lock().unwrap_or_else(|e| e.into_inner()).link = link;
            broadcast(&connections, &ControlMessage::LinkParams { link }, 0);
        }
    });
}

fn spawn_local_outbound_loop(
    outbound_rx: std::sync::mpsc::Receiver<ControlMessage>,
    state: Arc<Mutex<RoomState>>,
    connections: Arc<Mutex<Vec<(u8, TcpStream)>>>,
) {
    std::thread::spawn(move || {
        for msg in outbound_rx {
            match &msg {
                ControlMessage::RomChanged { rom_fingerprint } => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    let self_id = guard.self_id;
                    if let Some(p) = guard.players.iter_mut().find(|p| p.id == self_id) {
                        p.rom_fingerprint = *rom_fingerprint;
                    }
                    drop(guard);
                    broadcast(&connections, &player_list_message(&state), 0);
                }
                ControlMessage::MpReady { ready } => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    let self_id = guard.self_id;
                    if let Some(p) = guard.players.iter_mut().find(|p| p.id == self_id) {
                        p.mp_ready = *ready;
                    }
                    drop(guard);
                    broadcast(&connections, &player_list_message(&state), 0);
                }
                ControlMessage::LinkParams { link } => {
                    state.lock().unwrap_or_else(|e| e.into_inner()).link = *link;
                    broadcast(&connections, &msg, 0);
                }
                ControlMessage::Leave => {
                    state.lock().unwrap_or_else(|e| e.into_inner()).left = true;
                    return;
                }
                _ => {}
            }
        }
    });
}

fn spawn_client_reader_loop(
    mut stream: TcpStream,
    state: Arc<Mutex<RoomState>>,
    peers: Arc<PeerTable>,
    hints: Arc<SharedHints>,
    host_ip: IpAddr,
    mp_port: u16,
    shutdown: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        loop {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            match wire::read_framed(&mut stream) {
                Ok((_, ControlMessage::PlayerList { players })) => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.players = players
                        .into_iter()
                        .map(|p| PlayerView {
                            id: p.id,
                            name: p.name,
                            rom_fingerprint: p.rom_fingerprint,
                            is_host: p.is_host,
                            mp_ready: p.mp_ready,
                            rtt_ms: p.rtt_ms,
                            fps_x10: p.fps_x10,
                        })
                        .collect();
                    drop(guard);
                    // The host is always id 0 by protocol convention; the
                    // MP relay address for it never changes for the
                    // lifetime of this connection.
                    peers.set(vec![(0, SocketAddr::new(host_ip, mp_port))]);
                }
                Ok((_, ControlMessage::LinkParams { link })) => {
                    hints.set(link.to_hints());
                    state.lock().unwrap_or_else(|e| e.into_inner()).link = link;
                }
                Ok((_, ControlMessage::Reject { reason })) => {
                    state.lock().unwrap_or_else(|e| e.into_inner()).last_error =
                        Some(reject_message(reason));
                    return;
                }
                Ok(_) => {}
                Err(_) => {
                    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.last_error = Some("connection to host lost".to_owned());
                    guard.left = true;
                    return;
                }
            }
        }
    });
}

fn spawn_client_outbound_loop(
    outbound_rx: std::sync::mpsc::Receiver<ControlMessage>,
    write_half: Arc<Mutex<TcpStream>>,
    self_id: u8,
    shutdown: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        for msg in outbound_rx {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            let mut guard = write_half.lock().unwrap_or_else(|e| e.into_inner());
            if wire::write_framed(&mut *guard, &msg, self_id).is_err() {
                return;
            }
            if matches!(msg, ControlMessage::Leave) {
                return;
            }
        }
    });
}
