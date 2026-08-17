//! Headless proof that the core and the bindings work, independent of any
//! rendering this crate does.
//!
//! Run it before blaming the blit: if this reports a framebuffer with content,
//! a black window is this crate's fault; if it reports an all-black
//! framebuffer, the window is honest and the problem is upstream of egui.

use std::path::Path;

use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::emu::Emu;

/// Boot `rom`, run `frames` frames, and report progress and framebuffer
/// content. When `dump` is set, both screens are also written there as
/// `<dump>_top.png` / `<dump>_bottom.png`, converted exactly the way the
/// window's blit converts them — so a picture that looks right on disk means
/// the colour handling is right in the window too.
///
/// Returns a process exit code: 0 when the run produced a non-black picture.
pub fn run(rom: &Path, frames: u32, dump: Option<&str>) -> i32 {
    let mut emu = match Emu::boot(rom) {
        Ok(emu) => emu,
        Err(e) => {
            eprintln!("selftest: {e}");
            return 1;
        }
    };
    println!("selftest: booted {}", rom.display());

    let start = std::time::Instant::now();
    let mut stopped_at = None;
    for frame in 0..frames {
        if emu.nds.run_frame() == 0 {
            stopped_at = Some(frame);
            break;
        }
    }
    let elapsed = start.elapsed();

    if let Some(frame) = stopped_at {
        eprintln!("selftest: core stopped at frame {frame}");
        return 1;
    }

    let ran = frames;
    println!(
        "selftest: {ran} frames in {elapsed:.2?} = {:.1} fps",
        f64::from(ran) / elapsed.as_secs_f64(),
    );

    let Some((top, bottom)) = emu.nds.framebuffers() else {
        eprintln!("selftest: no framebuffer produced after {ran} frames");
        return 1;
    };
    let total = SCREEN_WIDTH * SCREEN_HEIGHT;
    let lit = |fb: &[u32]| fb.iter().filter(|&&px| px & 0x00FF_FFFF != 0).count();
    let (lit_top, lit_bottom) = (lit(top), lit(bottom));
    println!("selftest: non-black pixels  top {lit_top}/{total}  bottom {lit_bottom}/{total}",);
    // A distinct pixel count is the cheap check that the two screens are not
    // the same buffer read twice, which is what a mixed-up pointer pair looks
    // like once it reaches the window.
    println!("selftest: distinct colours top {}", distinct(top));

    if let Some(prefix) = dump {
        for (name, fb) in [("top", top), ("bottom", bottom)] {
            let path = format!("{prefix}_{name}.png");
            let rgb: Vec<u8> =
                fb.iter().flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8]).collect();
            match image::save_buffer(
                &path,
                &rgb,
                SCREEN_WIDTH as u32,
                SCREEN_HEIGHT as u32,
                image::ExtendedColorType::Rgb8,
            ) {
                Ok(()) => println!("selftest: wrote {path}"),
                Err(e) => eprintln!("selftest: failed to write {path}: {e}"),
            }
        }
    }

    if lit_top == 0 && lit_bottom == 0 {
        eprintln!("selftest: both screens are entirely black");
        return 1;
    }
    println!("selftest: OK");
    0
}

/// How many different colours a framebuffer holds, capped so a full screen of
/// noise does not build a huge set.
fn distinct(fb: &[u32]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for &px in fb {
        seen.insert(px);
        if seen.len() >= 4096 {
            break;
        }
    }
    seen.len()
}
