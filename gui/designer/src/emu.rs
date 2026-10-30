//! The running DS: boot, real-time pacing, keyboard/stylus input, screen
//! textures, and the FPGA kept in Slot-2 across resets.
//!
//! ```text
//!  update() each repaint
//!    keys + stylus -> Console
//!    frames owed by wall-clock time (<= MAX_BATCH) -> emulate_frame()
//!    new frame -> both screen textures re-uploaded
//! ```
//!
//! Pacing follows gui/egui (`LunarisApp::emulate_batch`).

use std::path::PathBuf;

use ds_fpga::FpgaSlot;
use eframe::egui;
use lunaris_gui_common::{
    config::Config,
    console::{Console, HEIGHT, Key, WIDTH},
    framebuffer::argb8888_to_rgba8,
};

/// The designer shares the main front end's instance (saves, BIOS paths).
const INSTANCE: u8 = 0;

/// Fixed keyboard map (host key -> DS button).
const KEYMAP: [(egui::Key, Key); 12] = [
    (egui::Key::X, Key::A),
    (egui::Key::Z, Key::B),
    (egui::Key::S, Key::X),
    (egui::Key::A, Key::Y),
    (egui::Key::Q, Key::L),
    (egui::Key::W, Key::R),
    (egui::Key::Enter, Key::Start),
    (egui::Key::Backspace, Key::Select),
    (egui::Key::ArrowUp, Key::Up),
    (egui::Key::ArrowDown, Key::Down),
    (egui::Key::ArrowLeft, Key::Left),
    (egui::Key::ArrowRight, Key::Right),
];

pub struct Emu {
    pub nds: Console,
    pub config: Config,
    pub paused: bool,
    /// Emulated frames since boot.
    pub frames: u64,
    pub fps: f64,
    frame_accum: f32,
    last_tick: std::time::Instant,
    fps_frames: u32,
    fps_since: std::time::Instant,
    /// Top, bottom.
    pub screens: [egui::TextureHandle; 2],
    screens_dirty: bool,
    /// The stylus position for the next frames (`None`: lifted).
    pub stylus: Option<(usize, usize)>,
    /// Buttons held with the mouse on the drawn console.
    pub mouse_keys: Vec<Key>,
}

impl Emu {
    const NDS_FPS: f32 = 59.8261;
    const MAX_BATCH: u32 = 4;

    pub fn new(ctx: &egui::Context, rom: Option<PathBuf>) -> Self {
        let mut config = Config::load_for_instance(INSTANCE);
        if let Some(rom) = rom {
            config.last_rom_path = Some(rom);
        }
        let nds = match &config.last_rom_path {
            Some(path) if path.exists() => lunaris_gui_common::loader::load_rom(&config),
            _ => Console::placeholder(),
        };
        let blank =
            egui::ColorImage::new([WIDTH, HEIGHT], vec![egui::Color32::BLACK; WIDTH * HEIGHT]);
        let tex = |name| ctx.load_texture(name, blank.clone(), egui::TextureOptions::NEAREST);
        Self {
            nds,
            config,
            paused: false,
            frames: 0,
            fps: 0.0,
            frame_accum: 0.0,
            last_tick: std::time::Instant::now(),
            fps_frames: 0,
            fps_since: std::time::Instant::now(),
            screens: [tex("top"), tex("bottom")],
            screens_dirty: true,
            stylus: None,
            mouse_keys: Vec::new(),
        }
    }

    pub fn rom_name(&self) -> String {
        match (&self.config.last_rom_path, self.nds.has_rom()) {
            (Some(p), true) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            }
            _ => "no ROM".to_owned(),
        }
    }

    /// Boots `path` (or reboots the current ROM), keeping the Slot-2 device.
    pub fn boot(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path {
            self.config.last_rom_path = Some(path);
        }
        self.nds.flush_save();
        let slot2 = self.nds.emulator_mut().slot2.insert(None);
        self.nds = lunaris_gui_common::loader::load_rom(&self.config);
        self.nds.emulator_mut().slot2.insert(slot2);
        self.config.save_for_instance(INSTANCE);
        self.frames = 0;
        self.paused = false;
        self.screens_dirty = true;
    }

    pub fn open_rom_dialog(&mut self) {
        let dialog = rfd::AsyncFileDialog::new().add_filter("NDS ROM", &["nds"]);
        let dialog = match self.config.last_rom_path.as_ref().and_then(|p| p.parent()) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        if let Some(file) = pollster::block_on(dialog.pick_file()) {
            self.boot(Some(file.path().to_path_buf()));
        }
    }

    /// The FPGA in Slot-2, if one is inserted.
    pub fn fpga(&mut self) -> Option<&mut FpgaSlot> {
        self.nds.emulator_mut().slot2.device_mut()?.as_any_mut().downcast_mut::<FpgaSlot>()
    }

    pub fn set_fpga_inserted(&mut self, inserted: bool) {
        let slot2 = &mut self.nds.emulator_mut().slot2;
        if inserted && slot2.device().is_none() {
            let mut fpga = FpgaSlot::default();
            if let Err(e) = fpga.add_spec(ds_fpga::example_timer()) {
                log::error!("{e}");
            }
            slot2.insert(Some(Box::new(fpga)));
        } else if !inserted {
            slot2.insert(None);
        }
    }

    pub fn update(&mut self, ctx: &egui::Context, keyboard: bool) {
        ctx.input(|i| {
            for (host, key) in KEYMAP {
                if (keyboard && i.key_down(host)) || self.mouse_keys.contains(&key) {
                    self.nds.press_key(key);
                } else {
                    self.nds.release_key(key);
                }
            }
        });

        let dt = self.last_tick.elapsed().as_secs_f32().min(0.25);
        self.last_tick = std::time::Instant::now();
        if self.paused || !self.nds.has_rom() {
            self.frame_accum = 0.0;
        } else {
            match self.stylus {
                Some((x, y)) => self.nds.press_screen(x, y),
                None => self.nds.release_screen(),
            }
            self.frame_accum += dt * Self::NDS_FPS * self.config.emu_speed;
            let batch = (self.frame_accum.max(0.0) as u32).min(Self::MAX_BATCH);
            for _ in 0..batch {
                self.nds.emulate_frame();
            }
            self.frame_accum = (self.frame_accum - batch as f32).min(1.0);
            self.frames += u64::from(batch);
            self.fps_frames += batch;
            self.screens_dirty |= batch > 0;
        }

        let elapsed = self.fps_since.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.fps = f64::from(self.fps_frames) / elapsed;
            self.fps_frames = 0;
            self.fps_since = std::time::Instant::now();
        }

        if std::mem::take(&mut self.screens_dirty) {
            for (tex, pixels) in self.screens.iter_mut().zip(self.nds.get_screens()) {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [WIDTH, HEIGHT],
                    &argb8888_to_rgba8(pixels),
                );
                tex.set(image, egui::TextureOptions::NEAREST);
            }
        }
    }

    /// Maps a pointer inside `rect` (where the bottom screen is drawn) to
    /// touch coordinates.
    pub fn touch_at(rect: egui::Rect, pos: egui::Pos2) -> Option<(usize, usize)> {
        rect.contains(pos).then(|| {
            let x = (pos.x - rect.min.x) / rect.width() * WIDTH as f32;
            let y = (pos.y - rect.min.y) / rect.height() * HEIGHT as f32;
            ((x as usize).min(WIDTH - 1), (y as usize).min(HEIGHT - 1))
        })
    }

    pub fn shutdown(&mut self) {
        self.nds.flush_save();
        self.config.save_for_instance(INSTANCE);
    }
}
