//! The Lunaris backend: this repository's `nds-core`.
//!
//! The core plays its own sound (its SPU feeds a `cpal` stream it opens
//! itself), so [`Core::take_audio`] adds nothing and the volume is set on
//! the core.
use std::{any::Any, path::Path};

use lunaris_gui_common::{config::Config, framebuffer::abgr1555_to_rgba8};
use nds_core::{
    ArCode, CheatMap,
    nds::{Engine, GraphicsType, Key, NDS},
};

use super::{Boot, Core, DebugView, HdJob, Image, PostScale};

/// `Key` in `emu_thread::key` bit order (each key's value is its bit).
const KEYS: [Key; 12] = [
    Key::A,
    Key::B,
    Key::Select,
    Key::Start,
    Key::Right,
    Key::Left,
    Key::Up,
    Key::Down,
    Key::R,
    Key::L,
    Key::X,
    Key::Y,
];

pub fn boot(boot: &Boot) -> Result<Box<dyn Core>, String> {
    // The loader unwraps file reads; fail here with a message instead.
    std::fs::metadata(&boot.rom).map_err(|e| e.to_string())?;
    let save_dir = boot.save.parent().map(ToOwned::to_owned).unwrap_or_default();
    std::fs::create_dir_all(&save_dir).map_err(|e| e.to_string())?;
    let config = Config {
        last_rom_path: Some(boot.rom.clone()),
        save_dir,
        bios7_path: boot.settings.arm7_bios_path.clone(),
        bios9_path: boot.settings.arm9_bios_path.clone(),
        firmware_path: boot.settings.firmware_path.clone(),
        audio_volume: boot.volume * 100.0,
        ..Config::default()
    };
    // Same boot as the egui GUI: free BIOS / firmware fallback, a DeSmuME
    // `.dsv` adopted when there is no `.sav` yet.
    let mut nds = lunaris_gui_common::loader::load_rom(&config);
    // The emulator thread paces frames; the audio queue must not block it.
    nds.set_audio_sync(false);
    Ok(Box::new(Lunaris { nds, keys: 0 }))
}

struct Lunaris {
    nds: NDS,
    /// Buttons held at the last frame.
    keys: u32,
}

impl Core for Lunaris {
    fn run_frame(&mut self, keys: u32, touch: Option<(i32, i32)>) {
        let changed = keys ^ self.keys;
        for key in KEYS {
            let bit = 1 << key as u32;
            if changed & bit != 0 {
                if keys & bit != 0 {
                    self.nds.press_key(key);
                } else {
                    self.nds.release_key(key);
                }
            }
        }
        self.keys = keys;
        match touch {
            Some((x, y)) => self.nds.press_screen(x as usize, y as usize),
            None => self.nds.release_screen(),
        }
        self.nds.emulate_frame();
    }

    fn take_audio(&mut self, _out: &mut Vec<i16>) {}

    fn frame(&mut self, rgba: &mut Vec<u8>) {
        for screen in self.nds.get_screens() {
            rgba.extend_from_slice(&abgr1555_to_rgba8(screen));
        }
    }

    /// The core draws at 256x192 only: scaled afterwards (xBRZ as the egui
    /// GUI's Upscale setting, or pixels repeated).
    fn hd_job(&mut self, scale: usize, xbrz: bool) -> Option<Box<dyn HdJob>> {
        let mut rgba = Vec::with_capacity(256 * 384 * 4);
        self.frame(&mut rgba);
        Some(Box::new(PostScale { rgba, scale, xbrz }))
    }

    fn set_cheats(&mut self, codes: Vec<Vec<u32>>) {
        self.nds.set_enable_cheats(!codes.is_empty());
        let map: CheatMap = codes.into_iter().map(|code| ArCode { code, enabled: true }).collect();
        self.nds.set_cheat_map(map);
    }

    fn set_volume(&mut self, volume: f32) {
        self.nds.set_audio_volume(volume * 100.0);
    }

    fn flush_save(&mut self) -> Result<(), String> {
        self.nds.flush_save();
        Ok(())
    }

    fn rom_fingerprint(&self) -> Option<[u8; 16]> {
        Some(self.nds.rom_fingerprint().to_bytes())
    }

    fn set_link(&mut self, link: Option<Box<dyn Any + Send>>) -> Result<(), String> {
        let transport = match link {
            Some(link) => {
                let transport = link
                    .downcast::<lunaris_net::NetTransport>()
                    .map_err(|_| "Not a Lunaris LAN link".to_owned())?;
                Some(transport as Box<dyn nds_core::nds::MpTransport>)
            }
            None => None,
        };
        self.nds.set_mp_transport(transport);
        Ok(())
    }

    fn wifi_status(&self) -> Vec<String> {
        let d = self.nds.wifi_diag_snapshot();
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
        ]
    }

    fn debug_image(&mut self, view: DebugView) -> Option<Image> {
        let engine = |b: bool| if b { Engine::B } else { Engine::A };
        let kind = |obj: bool| if obj { GraphicsType::OBJ } else { GraphicsType::BG };
        let (pixels, w, h) = match view {
            DebugView::Palettes { engine_b, obj, extended, slot, palette } => {
                self.nds.render_palettes(
                    extended,
                    slot.into(),
                    palette.into(),
                    engine(engine_b),
                    kind(obj),
                )
            }
            DebugView::Maps { engine_b, bg } => self.nds.render_map(engine(engine_b), bg.into()),
            DebugView::Tiles { engine_b, obj, extended, bitmap, bpp8, slot, palette, offset } => {
                self.nds.render_tiles(
                    engine(engine_b),
                    kind(obj),
                    extended,
                    bitmap,
                    bpp8,
                    slot.into(),
                    palette.into(),
                    offset.into(),
                )
            }
            DebugView::Vram { bank, ignore_alpha } => {
                self.nds.render_bank(bank.into(), ignore_alpha)
            }
        };
        Some((abgr1555_to_rgba8(&pixels), w, h))
    }

    /// The egui GUI's format (`LNST` + zstd), so states move between them.
    fn save_state(&mut self, path: &Path) -> Result<(), String> {
        lunaris_gui_common::savestate::save_to_file(&mut self.nds, path).map_err(|e| e.to_string())
    }

    fn load_state(&mut self, path: &Path) -> Result<(), String> {
        lunaris_gui_common::savestate::load_from_file(&mut self.nds, path)
            .map_err(|e| e.to_string())
    }
}
