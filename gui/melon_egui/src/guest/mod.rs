//! The second console, running on a thread of its own.
//!
//! # Why a thread of its own
//!
//! DS local wireless is a round per frame: the host sends a CMD, the client
//! answers within microseconds of emulated time, and the host's reply
//! collection **blocks** until the answer arrives. Two consoles taking turns
//! on one thread cannot do that — the host closes the round before the client
//! has run at all (a measured session: 10515 CMDs, 281 replies collected, and
//! a communication error). So the second console runs concurrently, paced by
//! the same wall clock, and [`crate::mp`]'s blocking receives bridge the gap.
//!
//! # What crosses between the threads
//!
//! ```text
//!  UI thread (app)                       guest thread (run_loop.rs)
//!  ───────────────                       ─────────────────────────
//!  Guest::set_input ──── Input ────────→ keys + stylus for the next frames
//!  Guest::send ───────── Command queue ─→ perform_commands (orders.rs)
//!  Guest::set_paused ─── AtomicBool ────→ hold / run
//!  Guest::take_screens ←─ Output ─────── newest picture
//!  Guest::take_note ←──── Output ─────── "state saved", "stopped", ...
//!  drop(Guest) ───────── quit flag ─────→ flush save, exit
//! ```
//!
//! The console itself — every `melonds` call — stays on its own thread.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use melonds::{Cheat, SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::{emu::Emu, i18n::I18nKey as K, mp::Client, remote::RemoteHost};

mod command;
mod handle;
mod orders;
mod run_loop;

pub use command::Command;
pub(crate) use handle::Shared;
pub use handle::{Guest, Note};
pub(crate) use orders::perform_commands;
pub(crate) use run_loop::{Outcome, RunConfig, finish, run};
