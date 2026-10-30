//! The Lunaris backend: this repository's core (`core/*`, `ds_emu`), driven
//! through `lunaris_gui_common::console::Console` as the egui GUI drives it.
//!
//! Audio is collected for the frontend's own output ([`Core::take_audio`]);
//! frames above 1x come from the core's own HD renderer (3D at the internal
//! resolution, xBRZ for the 2D layers).
use std::{any::Any, path::Path};

use ds_common::emu_thread::key;
use ds_emu::Emulator;
use ds_gpu::{display::hires::HdFrame, engine_3d::raster::hires::Renderer};
use lunaris_gui_common::{
    cheat_map::ArCode,
    console::{Console, Engine, GraphicsType, Key},
    framebuffer::abgr1555_to_rgba8,
};

use super::{Boot, Core, DebugView, HdJob, Image, push_rgba};

/// `Key` in `emu_thread::key` bit order.
const KEYS: [(u32, Key); 12] = [
    (key::A, Key::A),
    (key::B, Key::B),
    (key::SELECT, Key::Select),
    (key::START, Key::Start),
    (key::RIGHT, Key::Right),
    (key::LEFT, Key::Left),
    (key::UP, Key::Up),
    (key::DOWN, Key::Down),
    (key::R, Key::R),
    (key::L, Key::L),
    (key::X, Key::X),
    (key::Y, Key::Y),
];

pub fn boot(boot: &Boot) -> Result<Box<dyn Core>, String> {
    let save_dir = boot.save.parent().map(ToOwned::to_owned).unwrap_or_default();
    std::fs::create_dir_all(&save_dir).map_err(|e| e.to_string())?;
    let mut emu = Box::new(Emulator::new());
    boot.settings.apply(&mut emu);
    emu.config.save_dir = Some(save_dir);
    // Same boot as the egui GUI: a DeSmuME `.dsv` beside the ROM is adopted
    // when there is no `.sav` yet.
    if !boot.save.exists()
        && let Ok(dsv) = std::fs::read(boot.rom.with_extension("dsv"))
    {
        let _ = std::fs::write(&boot.save, ds_emu::normalize_foreign_save(&dsv));
    }
    let console = Console::with_emulator(emu, &boot.rom, None)?;
    Ok(Box::new(Lunaris { console }))
}

struct Lunaris {
    console: Console,
}

impl Core for Lunaris {
    fn run_frame(&mut self, keys: u32, touch: Option<(i32, i32)>) {
        for (bit, key) in KEYS {
            if keys & bit != 0 {
                self.console.press_key(key);
            } else {
                self.console.release_key(key);
            }
        }
        match touch {
            Some((x, y)) => self.console.press_screen(x.max(0) as usize, y.max(0) as usize),
            None => self.console.release_screen(),
        }
        self.console.emulate_frame();
    }

    fn set_render_scale(&mut self, scale: usize, _xbrz: bool) {
        self.console.emulator_mut().gpu.set_render_scale(scale);
    }

    fn take_audio(&mut self, out: &mut Vec<i16>) {
        self.console.take_audio(out);
    }

    fn frame(&mut self, rgba: &mut Vec<u8>) {
        for screen in self.console.get_screens() {
            push_rgba(screen, rgba);
        }
    }

    fn hd_job(&mut self, scale: usize, xbrz: bool) -> Option<Box<dyn HdJob>> {
        let frame = self.console.emulator_mut().gpu.hd_frame();
        Some(Box::new(Hd { frame, scale, xbrz }))
    }

    fn set_cheats(&mut self, codes: Vec<Vec<u32>>) {
        self.console.set_enable_cheats(!codes.is_empty());
        let map = codes.into_iter().map(|code| ArCode { code, enabled: true }).collect();
        self.console.set_cheat_map(map);
    }

    fn flush_save(&mut self) -> Result<(), String> {
        self.console.emulator_mut().flush_save().map_err(|e| e.to_string())
    }

    fn set_link(&mut self, link: Option<Box<dyn Any + Send>>) -> Result<(), String> {
        let link = match link {
            Some(link) => {
                let link = link
                    .downcast::<ds_net::NetLink>()
                    .map_err(|_| "Not a melonDS-net link".to_owned())?;
                Some(link as Box<dyn ds_emu::MpLink>)
            }
            None => None,
        };
        self.console.set_mp_link(link);
        Ok(())
    }

    fn wifi_status(&self) -> Vec<String> {
        let d = self.console.wifi_diag_snapshot();
        let mark = |ok: bool| if ok { "OK" } else { "--" };
        let m = d.mac;
        vec![
            format!(
                "MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                m[0], m[1], m[2], m[3], m[4], m[5]
            ),
            format!("{} 1. RF channel {}", mark(d.channel != 0), d.channel),
            format!(
                "{} 2. Driver setup (mode reset {}, rx buffer {})",
                mark(d.mode_reset > 0 || d.rxbuf_cfg > 0),
                d.mode_reset,
                d.rxbuf_cfg
            ),
            format!(
                "{} 3. Transmitting (beacon {}, cmd {}, reply {})",
                mark(d.loc_tx + d.beacon_tx + d.cmd_tx + d.reply_tx + d.blank_reply_tx > 0),
                d.beacon_tx,
                d.cmd_tx,
                d.reply_tx
            ),
            format!("{} 4. Receiving (accepted {})", mark(d.rx_accepted > 0), d.rx_accepted),
            format!(
                "{} 5. Classified (beacon {}, cmd {}, ack {}, reply {})",
                mark(d.rxflags_beacon + d.rxflags_cmd + d.rxflags_ack + d.rxflags_reply > 0),
                d.rxflags_beacon,
                d.rxflags_cmd,
                d.rxflags_ack,
                d.rxflags_reply
            ),
            format!(
                "{} 6. Associated (is_mp {}, client {}, aid {})",
                mark(d.is_mp),
                d.is_mp,
                d.is_mp_client,
                d.aid
            ),
            d.verdict(),
        ]
    }

    fn debug_image(&mut self, view: DebugView) -> Option<Image> {
        let engine = |b: bool| if b { Engine::B } else { Engine::A };
        let kind = |obj: bool| if obj { GraphicsType::OBJ } else { GraphicsType::BG };
        let c = &self.console;
        let (pixels, w, h) = match view {
            DebugView::Palettes { engine_b, obj, extended, slot, palette } => c.render_palettes(
                extended,
                slot.into(),
                palette.into(),
                engine(engine_b),
                kind(obj),
            ),
            DebugView::Maps { engine_b, bg } => c.render_map(engine(engine_b), bg.into()),
            DebugView::Tiles { engine_b, obj, extended, bitmap, bpp8, slot, palette, offset } => c
                .render_tiles(
                    engine(engine_b),
                    kind(obj),
                    extended,
                    bitmap,
                    bpp8,
                    slot.into(),
                    palette.into(),
                    offset.into(),
                ),
            DebugView::Vram { bank, ignore_alpha } => c.render_bank(bank.into(), ignore_alpha),
        };
        Some((abgr1555_to_rgba8(&pixels), w, h))
    }

    /// The egui GUI's format (`LNST` + zstd), so states move between them.
    fn save_state(&mut self, path: &Path) -> Result<(), String> {
        lunaris_gui_common::savestate::save_to_file(&self.console, path).map_err(|e| e.to_string())
    }

    fn load_state(&mut self, path: &Path) -> Result<(), String> {
        lunaris_gui_common::savestate::load_from_file(&mut self.console, path)
            .map_err(|e| e.to_string())
    }
}

/// A copy of the frame's inputs, upscaled beside emulation.
struct Hd {
    frame: HdFrame,
    scale: usize,
    xbrz: bool,
}

thread_local! {
    /// The HD worker's 3D renderer, kept between frames.
    static RENDERER: std::cell::RefCell<Renderer> = std::cell::RefCell::new(Renderer::default());
}

impl HdJob for Hd {
    fn render(self: Box<Self>, rgba: &mut Vec<u8>) -> usize {
        let mut pixels = Vec::new();
        let mut width = 256;
        RENDERER.with_borrow_mut(|renderer| {
            for upper in [true, false] {
                width = self.frame.screen(renderer, upper, self.scale, self.xbrz, &mut pixels);
                push_rgba(&pixels, rgba);
            }
        });
        width
    }
}
