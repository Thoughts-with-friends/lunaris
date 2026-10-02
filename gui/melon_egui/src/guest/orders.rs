//! Carrying out the second console's menu commands, between frames.
//!
//! The UI thread only *queues* a [`Command`]; this runs on the console's own
//! thread at the top of every loop pass, because every `melonds` call for this
//! console must happen here. Results come back to the UI as notes
//! ([`Shared::say`]).

use super::*;

/// Perform every command queued since the last pass.
///
/// The queue is emptied under its lock and acted on outside it, so a slow
/// savestate does not block the UI thread from posting the next command.
pub(crate) fn perform_commands(
    emu: &mut Emu,
    shared: &Shared,
    undo: &mut Option<Vec<u8>>,
    stepping: &mut u32,
) -> Outcome {
    let queued: Vec<Command> = match shared.commands.lock() {
        Ok(mut commands) => std::mem::take(&mut *commands),
        Err(_) => return Outcome::Continue,
    };
    for command in queued {
        match command {
            Command::Reset => {
                emu.nds.boot();
                shared.say("reset".to_owned());
            }
            Command::FrameStep => *stepping += 1,
            Command::SaveState(slot, path) => {
                let Some(path) = path.or_else(|| slot.map(|slot| emu.state_path(slot))) else {
                    continue;
                };
                shared.say(match emu.save_state_to(&path) {
                    Ok(bytes) => format!(
                        "state saved to {} ({:.1} MiB)",
                        path.display(),
                        bytes as f64 / (1024.0 * 1024.0)
                    ),
                    Err(error) => format!("save state failed: {error}"),
                });
            }
            Command::LoadState(slot, path) => {
                let Some(path) = path.or_else(|| slot.map(|slot| emu.state_path(slot))) else {
                    continue;
                };
                shared.say(match emu.load_state_from(&path) {
                    Ok(before) => {
                        *undo = before;
                        format!("state loaded from {}", path.display())
                    }
                    Err(error) => format!("load state failed: {error}"),
                });
            }
            Command::UndoStateLoad => {
                let Some(before) = undo.take() else {
                    shared.say("nothing to undo".to_owned());
                    continue;
                };
                shared.say(match emu.nds.load_state(&before) {
                    Ok(()) => "state load undone".to_owned(),
                    Err(error) => format!("undo failed: {error}"),
                });
            }
            Command::ImportSave(data) => {
                shared.say(match emu.import_save(&data) {
                    Ok(()) => "save imported; console rebooted".to_owned(),
                    Err(error) => format!("import failed: {error}"),
                });
            }
            Command::SetCheats(cheats) => emu.nds.set_cheats(cheats.as_slice()),
            Command::FlushSave => emu.flush_save(),
            Command::SetClock(clock) => emu.set_clock(clock),
            Command::Stop => {
                emu.flush_save();
                finish(shared, "stopped".to_owned());
                return Outcome::Stopped;
            }
        }
    }
    Outcome::Continue
}
