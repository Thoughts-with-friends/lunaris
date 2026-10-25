//! Menu actions: what needs this window (file pickers, quitting, window
//! size, dialogs) is done here, the rest by `Frontend::apply`.
use ds_common::{frontend::lock, menu::Action, view::place};
use freya::{prelude::*, winit::dpi::LogicalSize};
use rfd::AsyncFileDialog;

use crate::{frontend::Command, menu::Act, state::Ctx};

/// Height of the titlebar and the status bar around the screens (Screen
/// size resizes the window so the screens get N x their size).
pub const TITLEBAR_HEIGHT: f32 = 44.0;
pub const STATUSBAR_HEIGHT: f32 = 30.0;
/// Padding around the screen area.
pub const SCREEN_PADDING: f32 = 12.0;

/// Carries out `action`, then refreshes the shared state.
pub fn run(ctx: &Ctx, action: impl Into<Act>) {
    let action = match action.into() {
        Act::Ds(action) => action,
        Act::SaveState(slot) => return save_state(ctx, slot),
        Act::LoadState(slot) => return load_state(ctx, slot),
        Act::SaveStateFile => return state_file(ctx.clone(), false),
        Act::LoadStateFile => return state_file(ctx.clone(), true),
        Act::ExportSave => return export_save(ctx.clone()),
        Act::UndoStateLoad => return done(ctx, ctx.front.undo_state_load()),
        Act::Debug(tab) => {
            let mut debug = ctx.debug;
            let next = (*debug.peek() != Some(tab)).then_some(tab);
            if next.is_none() {
                ctx.front.shared.set_debug_view(None);
            }
            debug.set(next);
            return ctx.refresh();
        }
    };
    match action {
        Action::OpenRom => pick_rom(ctx.clone()),
        Action::ImportSavefile => import_savefile(ctx.clone()),
        Action::Quit => quit(ctx),
        Action::Open(window) => {
            let mut dialog = ctx.dialog;
            dialog.set(Some(window));
        }
        Action::ScreenSize(n) => {
            apply(ctx, action);
            resize_window(ctx, f32::from(n));
        }
        action => apply(ctx, action),
    }
}

fn apply(ctx: &Ctx, action: Action) {
    if let Err(e) = ctx.front.apply(action) {
        ctx.show_error(e);
    }
    ctx.refresh();
}

/// Stops the game (its save is written) and closes the window.
pub fn quit(ctx: &Ctx) {
    ctx.front.stop();
    Platform::get().close_window(Platform::window_id());
}

fn pick_rom(ctx: Ctx) {
    let dir = lock(&ctx.front.config).rom_dir.clone();
    spawn_in_window(async move {
        let mut dialog = AsyncFileDialog::new().add_filter("Nintendo DS ROM", &["nds"]);
        if dir.is_dir() {
            dialog = dialog.set_directory(&dir);
        }
        if let Some(file) = dialog.pick_file().await {
            apply(&ctx, Action::OpenRomPath(file.path().to_owned()));
        }
    });
}

/// File > Import savefile: stop, back the old save up, copy, restart.
fn import_savefile(ctx: Ctx) {
    let Some(rom) = lock(&ctx.front.rom).clone() else {
        return;
    };
    spawn_in_window(async move {
        let Some(file) =
            AsyncFileDialog::new().add_filter("DS save", &["sav", "dsv", "bin"]).pick_file().await
        else {
            return;
        };
        ctx.front.stop();
        let result = ctx
            .front
            .instance
            .import_save(&rom, file.path())
            .map_err(|e| format!("Import failed: {e}"))
            .and_then(|_| ctx.front.start(&rom));
        if let Err(e) = result {
            ctx.show_error(e);
        }
        ctx.refresh();
    });
}

/// The result of a command, as an error or a refresh.
fn done(ctx: &Ctx, result: Result<(), String>) {
    if let Err(e) = result {
        ctx.show_error(e);
    }
    ctx.refresh();
}

/// File > Save state > `slot`.
fn save_state(ctx: &Ctx, slot: u8) {
    let result = ctx
        .front
        .state_path(slot)
        .ok_or_else(|| "No game is running".to_owned())
        .and_then(|path| ctx.front.command(Command::SaveState(path)).map(drop));
    done(ctx, result.map_err(|e| format!("Save state {slot}: {e}")));
}

/// File > Load state > `slot`.
fn load_state(ctx: &Ctx, slot: u8) {
    let result = ctx
        .front
        .state_path(slot)
        .ok_or_else(|| "No game is running".to_owned())
        .and_then(|path| ctx.front.load_state(path));
    done(ctx, result.map_err(|e| format!("Load state {slot}: {e}")));
}

/// File > Save state / Load state > to / from file...
fn state_file(ctx: Ctx, load: bool) {
    let dir = ctx.front.states_dir();
    let name = lock(&ctx.front.rom)
        .as_deref()
        .and_then(|r| r.file_stem())
        .map_or_else(|| "state".to_owned(), |s| s.to_string_lossy().into_owned());
    spawn_in_window(async move {
        let _ = std::fs::create_dir_all(&dir);
        let dialog = AsyncFileDialog::new().set_directory(&dir);
        let picked = if load {
            dialog.pick_file().await
        } else {
            dialog.set_file_name(format!("{name}.state")).save_file().await
        };
        let Some(file) = picked else {
            return;
        };
        let path = file.path().to_owned();
        let result = if load {
            ctx.front.load_state(path)
        } else {
            ctx.front.command(Command::SaveState(path)).map(drop)
        };
        done(&ctx, result);
    });
}

/// File > Export savefile...: the game's save written now, then copied.
fn export_save(ctx: Ctx) {
    let Some(rom) = lock(&ctx.front.rom).clone() else {
        return;
    };
    spawn_in_window(async move {
        let name = rom.with_extension("sav").file_name().map(|n| n.to_string_lossy().into_owned());
        let Some(file) = AsyncFileDialog::new()
            .add_filter("DS save", &["sav"])
            .set_file_name(name.unwrap_or_default())
            .save_file()
            .await
        else {
            return;
        };
        done(&ctx, ctx.front.export_save(file.path()));
    });
}

/// View > Screen size: the window grows so the screens are `n` x native.
fn resize_window(ctx: &Ctx, n: f32) {
    let view = lock(&ctx.front.config).view.clone();
    let natural = place(&view, 1.0, 1.0);
    let w = natural.natural_w.mul_add(n, SCREEN_PADDING * 2.0);
    let h = natural
        .natural_h
        .mul_add(n, SCREEN_PADDING.mul_add(2.0, TITLEBAR_HEIGHT + STATUSBAR_HEIGHT));
    Platform::get().with_window(Platform::window_id(), move |window| {
        let _ = window.request_inner_size(LogicalSize::new(f64::from(w), f64::from(h)));
    });
}
