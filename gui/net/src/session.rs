//! melonDS LAN session control (host / join / leave).
//!
//! ```text
//!  LanSession::new()      ds_net::init(Lan)   (before any console runs)
//!  host(name)             lan::host           immediate
//!  join(name, addr)       worker: lan::join   up to ~5 s per try
//!    busy() == true       the caller must not run consoles meanwhile:
//!                         the LAN backend is not thread-safe
//!  tick(running)          worker finished? -> Joined / Failed
//!                         no console running -> ds_net::process()
//!                         (keeps ENet connections alive)
//!  leave()                lan::leave
//! ```

use std::{
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ds_net::{
    MpBackend,
    lan::{self, LanError, Player},
};

/// Players per hosted session.
pub const MAX_PLAYERS: u8 = 16;

/// Join attempts: a host only services ENet while its window repaints, so a
/// first try can miss it.
const JOIN_TRIES: usize = 3;

/// How often the player list is refreshed.
const PLAYERS_REFRESH: Duration = Duration::from_millis(500);

/// Where the session stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// No session.
    Offline,
    /// Hosting; clients may join.
    Hosting,
    /// Connecting to the host at this address.
    Joining(String),
    /// In the session hosted at this address.
    Joined(String),
    /// The last host/join attempt failed.
    Failed(String),
}

impl SessionState {
    /// Hosting or joined.
    pub const fn in_session(&self) -> bool {
        matches!(self, Self::Hosting | Self::Joined(_))
    }
}

/// The process's LAN session (melonDS `LAN`; one per process).
#[derive(Debug)]
pub struct LanSession {
    state: SessionState,
    worker: Option<JoinHandle<Result<(), LanError>>>,
    players: Vec<Player>,
    players_at: Option<Instant>,
    /// Bumped whenever the session starts or ends, so the caller knows to
    /// re-attach its consoles' links (melonDS only puts a console on a LAN
    /// medium that existed when it powered its Wi-Fi on).
    generation: u64,
}

impl Default for LanSession {
    fn default() -> Self {
        Self::new()
    }
}

impl LanSession {
    /// Selects melonDS's LAN backend. Call before any console runs.
    pub fn new() -> Self {
        if ds_net::backend() != MpBackend::Lan {
            ds_net::init(MpBackend::Lan);
        }
        Self {
            state: SessionState::Offline,
            worker: None,
            players: Vec::new(),
            players_at: None,
            generation: 0,
        }
    }

    pub const fn state(&self) -> &SessionState {
        &self.state
    }

    /// A join is in progress: do not run consoles or call into `ds_net`.
    pub const fn busy(&self) -> bool {
        self.worker.is_some()
    }

    /// Changes whenever a session starts or ends.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Players of the current session (refreshed by [`Self::tick`]).
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Hosts a session for up to [`MAX_PLAYERS`] consoles.
    ///
    /// # Errors
    /// When UDP 7064 cannot be bound (another host on this machine?).
    pub fn host(&mut self, player: &str) -> Result<(), LanError> {
        if self.busy() {
            return Err(LanError::Failed);
        }
        self.leave();
        match lan::host(player, MAX_PLAYERS) {
            Ok(()) => {
                log::info!("LAN: hosting as {player}");
                self.state = SessionState::Hosting;
                self.started();
                Ok(())
            }
            Err(e) => {
                log::error!("LAN: host failed: {e}");
                self.state = SessionState::Failed(format!("host: {e}"));
                Err(e)
            }
        }
    }

    /// Starts joining the session hosted at `address` (IP or host name; a
    /// `:port` suffix is ignored, the LAN port is fixed) on a worker thread;
    /// [`Self::tick`] reports the outcome.
    pub fn join(&mut self, player: &str, address: &str) {
        if self.busy() {
            return;
        }
        self.leave();
        let (player, host) = (player.to_owned(), crate::host_part(address));
        log::info!("LAN: joining {host} as {player}");
        self.worker = Some(std::thread::spawn(move || {
            (0..JOIN_TRIES)
                .map(|_| lan::join(&player, &host))
                .find(Result::is_ok)
                .unwrap_or(Err(LanError::Failed))
        }));
        self.state = SessionState::Joining(address.to_owned());
    }

    /// Leaves / closes the current session.
    pub fn leave(&mut self) {
        if self.busy() {
            return;
        }
        if self.state.in_session() {
            log::info!("LAN: leaving the session");
            lan::leave();
            self.generation += 1;
        }
        self.state = SessionState::Offline;
        self.players.clear();
        self.players_at = None;
    }

    /// Once per GUI frame. `console_running`: a console is emulated this
    /// frame (its link pumps the backend then).
    pub fn tick(&mut self, console_running: bool) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(worker) = self.worker.take()
        {
            let address = match &self.state {
                SessionState::Joining(address) => address.clone(),
                _ => String::new(),
            };
            match worker.join() {
                Ok(Ok(())) => {
                    log::info!("LAN: joined {address}");
                    self.state = SessionState::Joined(address);
                    self.started();
                }
                Ok(Err(e)) => {
                    log::error!("LAN: join {address} failed: {e}");
                    self.state = SessionState::Failed(format!("join {address}: {e}"));
                }
                Err(_) => self.state = SessionState::Failed("join: worker panicked".to_owned()),
            }
        }
        if self.busy() || !self.state.in_session() {
            return;
        }
        if !console_running {
            ds_net::process();
        }
        if self.players_at.is_none_or(|at| at.elapsed() >= PLAYERS_REFRESH) {
            self.players = lan::players();
            self.players_at = Some(Instant::now());
        }
    }

    fn started(&mut self) {
        self.generation += 1;
        self.players = lan::players();
        self.players_at = Some(Instant::now());
    }
}

impl Drop for LanSession {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.leave();
    }
}
