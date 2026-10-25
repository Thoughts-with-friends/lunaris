// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(feature = "release", windows_subsystem = "windows")]
//! freya frontend: the same menus, dialogs and features as the egui / Tauri
//! GUIs in a borderless, translucent window.
//!
//! ```text
//!  main: Instance + Settings + logging -> Frontend (frontend.rs) -> launch
//!        (the core is fixed at build time: feature lunaris / melon / cogi)
//!  App (root)
//!    ├─ Titlebar   logo, MenuBar (ds_common::menu), drag area, Appearance,
//!    │             minimize / maximize / close
//!    ├─ Screens    frames of the emulator thread, stylus   (or Welcome)
//!    ├─ StatusBar  state, game, fps, speed, resolution, instance
//!    └─ Dialogs    modal cards (ROM info, settings, cheats, ...)
//!  input.rs: keyboard + gamepads -> Shared::set_keys, status refresh
//!  close (✕, File > Quit, Alt+F4) -> Frontend::stop (writes the save)
//! ```
mod actions;
mod backend;
mod debug;
mod dialogs;
mod frontend;
mod input;
mod lan;
mod menu;
mod menubar;
mod panels;
mod screens;
mod state;
mod theme;
mod titlebar;
#[cfg(test)]
mod ui_tests;

use std::sync::Arc;

use ds_common::{Instance, Settings};
use freya::{borderless::BorderlessPlugin, prelude::*};

use crate::{
    debug::DebugPanel,
    dialogs::Dialogs,
    frontend::Frontend,
    panels::{StatusBar, Welcome},
    screens::Screens,
    state::Ctx,
    titlebar::Titlebar,
};

/// Corner radius of the window (drawn by the borderless plugin).
const WINDOW_RADIUS: f32 = 14.0;

/// The root component.
#[derive(Clone)]
struct App {
    front: Arc<Frontend>,
    /// A dialog open from the start (snapshot tests).
    dialog: Option<ds_common::menu::Window>,
}

impl PartialEq for App {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.front, &other.front)
    }
}

impl Component for App {
    fn render(&self) -> impl IntoElement {
        use_init_theme(theme::lunaris_theme);
        let front = Arc::clone(&self.front);
        let open_dialog = self.dialog;
        let ctx = use_provide_context(move || {
            let ctx = Ctx::new(front);
            ctx.refresh();
            let mut dialog = ctx.dialog;
            dialog.set(open_dialog);
            input::spawn_loop(ctx.clone());
            ctx
        });
        let opacity = ctx.look.read().opacity;
        let running = ctx.status.read().running;
        let debug_open = ctx.debug.read().is_some();
        let down_ctx = ctx.clone();
        let up_ctx = ctx.clone();
        let mut appearance = ctx.appearance;

        rect()
            .expanded()
            .corner_radius(WINDOW_RADIUS)
            .overflow(Overflow::Clip)
            .background(theme::rgba(theme::BODY, (opacity.clamp(0.3, 1.) * 255.) as u8))
            .border(Border::new().width(1.).fill(theme::hairline()))
            .color(theme::rgb(theme::TEXT))
            .font_family("Segoe UI")
            .on_global_key_down(move |e: Event<KeyboardEventData>| {
                input::on_key_down(&down_ctx, &e);
            })
            .on_global_key_up(move |e: Event<KeyboardEventData>| input::on_key_up(&up_ctx, &e))
            .on_press(move |_| appearance.set_if_modified(false))
            .child(
                rect()
                    .width(Size::fill())
                    .background(
                        LinearGradient::new()
                            .angle(90.)
                            .stop((theme::rgba(theme::ACCENT, 26), 0.))
                            .stop((theme::rgba(theme::ACCENT_2, 18), 100.)),
                    )
                    .child(Titlebar),
            )
            .child(rect().width(Size::fill()).height(Size::px(1.)).background(theme::hairline()))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::func(|ctx| {
                        Some(ctx.parent - actions::TITLEBAR_HEIGHT - actions::STATUSBAR_HEIGHT - 2.)
                    }))
                    .padding(actions::SCREEN_PADDING)
                    .horizontal()
                    .content(Content::Flex)
                    .spacing(actions::SCREEN_PADDING)
                    .child(rect().width(Size::flex(1.)).height(Size::fill()).child(if running {
                        Screens.into_element()
                    } else {
                        Welcome.into_element()
                    }))
                    .maybe_child(debug_open.then_some(DebugPanel)),
            )
            .child(rect().width(Size::fill()).height(Size::px(1.)).background(theme::hairline()))
            .child(StatusBar)
            .child(Dialogs)
    }
}

fn main() {
    let instance = Instance::open();
    let mut settings = Settings::load(&instance.settings_path());
    if settings.ensure_mac() {
        settings.save(&instance.settings_path());
    }
    if let Err(err) = ds_common::log::setup_logging_in(&instance.logs_dir()) {
        log::error!("Failed to start logging: {err}");
    }
    log::info!(
        "Started freya GUI on the {} core (instance folder: {})",
        backend::BACKEND,
        instance.root().display()
    );
    let title: &'static str =
        Box::leak(format!("Lunaris DS (instance{})", instance.number()).into_boxed_str());

    let front = Arc::new(Frontend::new(instance, settings));
    let app = App { front: Arc::clone(&front), dialog: None };
    let root = move || app.clone();
    let on_close = Arc::clone(&front);
    launch(
        LaunchConfig::new()
            .with_plugin(BorderlessPlugin::new().with_corner_radius(WINDOW_RADIUS))
            .with_window(
                WindowConfig::new(root)
                    .with_title(title)
                    .with_size(560., 560.)
                    .with_min_size(420., 360.)
                    .with_decorations(false)
                    .with_transparency(true)
                    .with_background(Color::TRANSPARENT)
                    // Alt+F4 and the like: write the save before closing.
                    .with_on_close(move |_, _| {
                        on_close.stop();
                        CloseDecision::Close
                    }),
            ),
    );
    // However the window went away, the save is written.
    front.stop();
}
