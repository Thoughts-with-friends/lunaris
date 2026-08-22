//! The second console, on a thread of its own.
//!
//! # Why it cannot share the first console's thread
//!
//! melonDS's local wireless is built for instances that run *concurrently*: its
//! own front end launches a second process. A wireless round is host sends CMD
//! → clients answer within microseconds of emulated time → host collects the
//! replies, and the host's collection **blocks** (melonDS `LocalMP::RecvReplies`
//! waits on a semaphore with a 25 ms timeout) precisely because the answer is
//! expected to arrive while it waits.
//!
//! Running both consoles alternately on one thread cannot satisfy that. The
//! host asks for replies in the same breath as it sends the CMD, before the
//! other console has executed a single cycle of that round; by the time the
//! client answers, the host has closed the round and reads the answer as stale.
//! That is exactly what a session produced: 10515 CMDs sent, 10512 replies sent
//! back, 281 collected, 1393 discarded as stale — and a communication error in
//! the game.
//!
//! So the second console runs here, on its own thread, paced by the same
//! wall clock as the first. Neither waits for the other's frame; they overlap,
//! and [`crate::mp`]'s blocking receives bridge the microseconds between them.
//!
//! # What crosses the boundary
//!
//! As little as possible: the keys and touch the UI thread sampled, the picture
//! the console last drew, and a note when something happens. The console itself
//! — and every `melonds` call — stays on this thread.

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

use crate::{emu::Emu, mp::Client, remote::RemoteHost};

mod command;
mod handle;
mod orders;
mod run_loop;

pub use command::Command;
pub use handle::Guest;
pub(crate) use handle::Shared;
pub(crate) use orders::perform_commands;
pub(crate) use run_loop::{Outcome, RunConfig, run};
