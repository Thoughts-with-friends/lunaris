//! Running a blocking job (a network handshake) off the UI thread.
//!
//! ```text
//!  start_lan / start_remote ── spawn(job) ──→ worker thread blocks on the
//!        │                                     handshake, then sends its result
//!        ▼ (stores the Receiver)
//!  every repaint: take(&mut slot)
//!        ├─ None            still waiting
//!        ├─ Some(Some(r))   finished: act on r
//!        └─ Some(None)      the thread died without answering
//! ```

use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Run `job` on a thread called `name`; its result arrives on the receiver.
pub(crate) fn spawn<T: Send + 'static>(
    name: &str,
    job: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<Receiver<T>> {
    let (sender, receiver) = channel();
    std::thread::Builder::new().name(name.to_owned()).spawn(move || {
        // A failed send means the UI dropped the receiver; nobody to tell.
        let _ = sender.send(job());
    })?;
    Ok(receiver)
}

/// Collect a worker's result, emptying `slot` once it has one (or died).
pub(crate) fn take<T>(slot: &mut Option<Receiver<T>>) -> Option<Option<T>> {
    let answer = match slot.as_ref()?.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => None,
    };
    *slot = None;
    Some(answer)
}
