//! Headless smoke test: loads a real ROM, runs several frames, and dumps the
//! upper/lower framebuffers as PNGs for manual visual inspection.
//!
//! Ignored by default since it depends on a ROM file present on the
//! developer's machine. Run explicitly with:
//! `cargo test -p lunaris_ds_emu --release --test headless_render -- --ignored --nocapture`
use lunaris_ds_emu::Emulator;
use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};

/// Converts the core's 0xAARRGGBB pixels to 0xAABBGGRR so they can be
/// written out as standard RGBA image bytes (same conversion the egui GUI
/// applies before handing frames to `ColorImage`).
fn argb_to_rgba_bytes(buffer: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(buffer.len() * 4);
    for &px in buffer {
        let a = (px >> 24) as u8;
        let r = (px >> 16) as u8;
        let g = (px >> 8) as u8;
        let b = px as u8;
        bytes.extend_from_slice(&[r, g, b, a]);
    }
    bytes
}

#[test]
#[ignore = "requires a real NDS ROM on disk"]
fn dump_frames_from_real_rom() {
    let rom_path = std::env::var("LUNARIS_TEST_ROM")
        .unwrap_or_else(|_| "D:/GAME/game/DS/Roms/イナズマイレブン3_スパーク.nds".to_string());
    let out_dir = std::env::var("LUNARIS_TEST_OUT_DIR")
        .unwrap_or_else(|_| std::env::temp_dir().display().to_string());

    let mut emu = Emulator::new();
    emu.load_rom(&rom_path).expect("ROM should load");

    const FRAME_COUNT: usize = 1800; // ~30 seconds of emulated time at 60fps
    const PIXELS: usize = PIXELS_PER_LINE * SCANLINES;
    let mut upper = vec![0_u32; PIXELS];
    let mut lower = vec![0_u32; PIXELS];

    for i in 0..FRAME_COUNT {
        emu.run();
        if i % 60 == 0 {
            emu.get_upper_frame(&mut upper);
            emu.get_lower_frame(&mut lower);
            let upper_path = format!("{out_dir}/lunaris_upper_{i:04}.png");
            let lower_path = format!("{out_dir}/lunaris_lower_{i:04}.png");
            image::save_buffer(
                &upper_path,
                &argb_to_rgba_bytes(&upper),
                PIXELS_PER_LINE as u32,
                SCANLINES as u32,
                image::ColorType::Rgba8,
            )
            .expect("failed to save upper frame");
            image::save_buffer(
                &lower_path,
                &argb_to_rgba_bytes(&lower),
                PIXELS_PER_LINE as u32,
                SCANLINES as u32,
                image::ColorType::Rgba8,
            )
            .expect("failed to save lower frame");
            println!("frame {i}: saved {upper_path} / {lower_path}");
        }
    }
}
