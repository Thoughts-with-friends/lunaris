//! Carrying out the menu commands, between frames.

use super::*;

/// Perform everything the second console's menu has asked for since the last
/// pass.
///
/// The queue is drained under its lock and acted on outside it, so that a
/// savestate — which takes a moment for a large cart — does not hold up a UI
/// thread that only wants to post the next command.
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
                let Some(path) = state_path(emu, slot, path) else { continue };
                let mut buffer = Vec::new();
                let outcome =
                    emu.nds.save_state(&mut buffer).map_err(|e| e.to_string()).and_then(|()| {
                        std::fs::write(&path, &buffer)
                            .map_err(|e| format!("cannot write {}: {e}", path.display()))
                    });
                shared.say(match outcome {
                    Ok(()) => format!(
                        "state saved to {} ({:.1} MiB)",
                        path.display(),
                        buffer.len() as f64 / (1024.0 * 1024.0)
                    ),
                    Err(error) => format!("save state failed: {error}"),
                });
            }
            Command::LoadState(slot, path) => {
                let Some(path) = state_path(emu, slot, path) else { continue };
                // Snapshot first, so the load can be taken back — the same undo
                // the first console offers.
                let mut before = Vec::new();
                let snapshot = emu.nds.save_state(&mut before).is_ok();
                let outcome = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))
                    .and_then(|buffer| emu.nds.load_state(&buffer).map_err(|e| e.to_string()));
                shared.say(match outcome {
                    Ok(()) => {
                        *undo = snapshot.then_some(before);
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
                shared.say("stopped".to_owned());
                if let Ok(mut out) = shared.output.lock() {
                    out.finished = true;
                }
                return Outcome::Stopped;
            }
        }
    }
    Outcome::Continue
}

/// Where a savestate goes: the explicit path if the menu asked for one,
/// otherwise the numbered slot in this instance's own `states` directory.
pub(crate) fn state_path(emu: &Emu, slot: Option<u8>, path: Option<PathBuf>) -> Option<PathBuf> {
    path.or_else(|| slot.map(|slot| emu.state_path(slot)))
}
