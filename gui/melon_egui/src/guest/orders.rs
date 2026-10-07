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
                shared.say(Note::new(K::ResetDone, &[]));
            }
            Command::FrameStep => *stepping += 1,
            Command::SaveState(slot, path) => {
                let Some(path) = path.or_else(|| slot.map(|slot| emu.state_path(slot))) else {
                    continue;
                };
                shared.say(match emu.save_state_to(&path) {
                    Ok(bytes) => {
                        let mib = format!("{:.1}", bytes as f64 / (1024.0 * 1024.0));
                        Note::new(K::StateSaved, &[&path.display(), &mib])
                    }
                    Err(error) => Note::new(K::StateSaveFailed, &[&error]),
                });
            }
            Command::LoadState(slot, path) => {
                let Some(path) = path.or_else(|| slot.map(|slot| emu.state_path(slot))) else {
                    continue;
                };
                shared.say(match emu.load_state_from(&path) {
                    Ok(before) => {
                        *undo = before;
                        Note::new(K::StateLoaded, &[&path.display()])
                    }
                    Err(error) => Note::new(K::StateLoadFailed, &[&error]),
                });
            }
            Command::UndoStateLoad => {
                let Some(before) = undo.take() else {
                    shared.say(Note::new(K::NothingToUndo, &[]));
                    continue;
                };
                shared.say(match emu.nds.load_state(&before) {
                    Ok(()) => Note::new(K::StateLoadUndone, &[]),
                    Err(error) => Note::new(K::UndoFailed, &[&error]),
                });
            }
            Command::ImportSave(data) => {
                shared.say(match emu.import_save(&data) {
                    Ok(()) => Note::new(K::GuestSaveImported, &[]),
                    Err(error) => Note::new(K::ImportFailed, &[&error]),
                });
            }
            Command::SetCheats(cheats) => emu.nds.set_cheats(cheats.as_slice()),
            Command::FlushSave => emu.flush_save(),
            Command::SetClock(clock) => emu.set_clock(clock),
            Command::Stop => {
                emu.flush_save();
                finish(shared, Note::new(K::GuestStoppedByUser, &[]));
                return Outcome::Stopped;
            }
        }
    }
    Outcome::Continue
}
