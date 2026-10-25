//! The Cogi backend: cogids-rs's `ds_emu`, the core this GUI came with.
use ds_common::emu_thread::key;
use ds_emu::Emulator;
use ds_gpu::{display::hires::HdFrame, engine_3d::raster::hires::Renderer};

use super::{Boot, Core, HdJob, push_rgba};

pub fn boot(boot: &Boot) -> Result<Box<dyn Core>, String> {
    let mut emu = Emulator::new();
    boot.settings.apply(&mut emu);
    emu.config.save_dir = boot.save.parent().map(ToOwned::to_owned);
    emu.load_rom(&boot.rom).map_err(|e| e.to_string())?;
    Ok(Box::new(Cogi { emu, pixels: Vec::new() }))
}

struct Cogi {
    emu: Emulator,
    pixels: Vec<u32>,
}

impl Core for Cogi {
    fn run_frame(&mut self, keys: u32, touch: Option<(i32, i32)>) {
        let emu = &mut self.emu;
        let on = |bit| keys & bit != 0;
        let k = &mut emu.key_input;
        k.button_a = on(key::A);
        k.button_b = on(key::B);
        k.select = on(key::SELECT);
        k.start = on(key::START);
        k.right = on(key::RIGHT);
        k.left = on(key::LEFT);
        k.up = on(key::UP);
        k.down = on(key::DOWN);
        k.button_r = on(key::R);
        k.button_l = on(key::L);
        emu.ext_key_in.button_x = on(key::X);
        emu.ext_key_in.button_y = on(key::Y);
        match touch {
            Some((x, y)) => emu.touchscreen_press(x, y),
            None => emu.touchscreen_press(0, 0xFFF),
        }
        emu.run();
    }

    fn set_render_scale(&mut self, scale: usize, _xbrz: bool) {
        self.emu.gpu.set_render_scale(scale);
    }

    fn take_audio(&mut self, out: &mut Vec<i16>) {
        self.emu.take_audio_samples(out);
    }

    fn frame(&mut self, rgba: &mut Vec<u8>) {
        for upper in [true, false] {
            self.emu.gpu.screen_hd(upper, 1, false, &mut self.pixels);
            push_rgba(&self.pixels, rgba);
        }
    }

    fn hd_job(&mut self, scale: usize, xbrz: bool) -> Option<Box<dyn HdJob>> {
        Some(Box::new(Hd { frame: self.emu.gpu.hd_frame(), scale, xbrz }))
    }

    fn set_cheats(&mut self, codes: Vec<Vec<u32>>) {
        self.emu.set_cheats(codes);
    }

    fn flush_save(&mut self) -> Result<(), String> {
        self.emu.flush_save().map_err(|e| e.to_string())
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
