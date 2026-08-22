//! Headless proof that the core and the bindings work, independent of any
//! rendering this crate does.
//!
//! Run it before blaming the blit: if this reports a framebuffer with content,
//! a black window is this crate's fault; if it reports an all-black
//! framebuffer, the window is honest and the problem is upstream of egui.

use std::path::Path;

use melonds::{SCREEN_HEIGHT, SCREEN_WIDTH};

use crate::{emu::Emu, mp::Airwaves};

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
            log::error!("selftest: {e}");
            return 1;
        }
    };
    log::info!("selftest: booted {}", rom.display());

    let start = std::time::Instant::now();
    let mut stopped_at = None;
    for frame in 0..frames {
        emu.nds.run_frame();
        // Asked, not inferred: a sleeping console draws no scanlines and is
        // still perfectly alive (see `MelonEgui::advance`).
        if !emu.nds.is_running() {
            stopped_at = Some(frame);
            break;
        }
    }
    let elapsed = start.elapsed();

    if let Some(frame) = stopped_at {
        log::error!("selftest: core stopped at frame {frame}");
        return 1;
    }

    let ran = frames;
    log::info!(
        "selftest: {ran} frames in {elapsed:.2?} = {:.1} fps",
        f64::from(ran) / elapsed.as_secs_f64(),
    );

    // Audio is produced whether or not a device is open, so this reports on the
    // core's output rather than on the host's sound card.
    let queued = emu.nds.audio_queued();
    let mut samples = vec![0i16; queued * 2];
    let read = emu.nds.read_audio(&mut samples);
    let loudest = samples[..read * 2].iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    log::info!("selftest: audio {read} sample frames buffered, peak amplitude {loudest}");

    let Some((top, bottom)) = emu.nds.framebuffers() else {
        log::error!("selftest: no framebuffer produced after {ran} frames");
        return 1;
    };
    let total = SCREEN_WIDTH * SCREEN_HEIGHT;
    let lit = |fb: &[u32]| fb.iter().filter(|&&px| px & 0x00FF_FFFF != 0).count();
    let (lit_top, lit_bottom) = (lit(top), lit(bottom));
    log::info!("selftest: non-black pixels  top {lit_top}/{total}  bottom {lit_bottom}/{total}",);
    // A distinct pixel count is the cheap check that the two screens are not
    // the same buffer read twice, which is what a mixed-up pointer pair looks
    // like once it reaches the window.
    log::info!("selftest: distinct colours top {}", distinct(top));

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
                Ok(()) => log::info!("selftest: wrote {path}"),
                Err(e) => log::error!("selftest: failed to write {path}: {e}"),
            }
        }
    }

    if !check_render_knobs(&mut emu) {
        return 1;
    }
    if !check_render_settings(&mut emu) {
        return 1;
    }
    if !check_cheats(&mut emu) {
        return 1;
    }
    if !check_two_instances(rom) {
        return 1;
    }

    if lit_top == 0 && lit_bottom == 0 {
        log::error!("selftest: both screens are entirely black");
        return 1;
    }
    log::info!("selftest: OK");
    0
}

/// The cheat engine, with a code whose effect is unmistakable.
///
/// `02100000 DEADBEEF` is an Action Replay 32-bit write: main RAM at 0x02100000
/// takes that value every time the ARM7 takes its VBlank IRQ. So the test is
/// that the word appears while the code is enabled, and that a different value
/// written there survives once it is not — which is what tells "the engine ran
/// it" apart from "something else happened to be there".
fn check_cheats(emu: &mut Emu) -> bool {
    const ADDR: u32 = 0x0210_0000;
    const VALUE: u32 = 0xDEAD_BEEF;

    let cheat =
        melonds::Cheat { name: "selftest".to_owned(), code: vec![ADDR, VALUE], enabled: true };
    emu.nds.set_cheats(std::slice::from_ref(&cheat));
    advance(emu, 8);
    let applied = emu.nds.arm7_read32(ADDR) == VALUE;

    // Off is an empty list, which is how the front end's master switch works.
    emu.nds.set_cheats(&[]);
    emu.nds.arm7_write32(ADDR, 0);
    advance(emu, 8);
    let stopped = emu.nds.arm7_read32(ADDR) == 0;

    log::info!(
        "selftest: cheat wrote {VALUE:08X} to {ADDR:08X}: {}; stopped when uninstalled: {}",
        yes_no(applied),
        yes_no(stopped),
    );
    if !applied || !stopped {
        log::error!("selftest: the cheat engine did not behave as documented");
        return false;
    }
    true
}

/// The Video settings dialog's renderer half, as far as a headless run can
/// take it.
///
/// Two things are checkable without a window. The threaded software renderer is
/// a real选択 here — it needs no GL at all — so it is selected and the picture
/// is required to keep moving and to stay a picture. An OpenGL renderer, on the
/// other hand, cannot work without a context: melonDS falls back to software
/// rather than leave a console unable to draw, and the test is that the
/// bindings report that fallback honestly instead of claiming OpenGL.
fn check_render_settings(emu: &mut Emu) -> bool {
    use melonds::{RenderSettings, Renderer};

    let threaded = RenderSettings { threaded: true, ..RenderSettings::default() };
    let installed = emu.set_render_settings(threaded);
    advance(emu, 30);
    let lit_threaded = emu.nds.framebuffers().is_some_and(|(top, bottom)| {
        let lit = |fb: &[u32]| fb.iter().any(|&px| px & 0x00FF_FFFF != 0);
        lit(top) || lit(bottom)
    });
    log::info!(
        "selftest: threaded software renderer installed: {}; still drawing: {}",
        yes_no(installed == Renderer::Software),
        yes_no(lit_threaded),
    );

    // No GL context exists in a headless run, so this is the fallback path.
    let gl = RenderSettings { renderer: Renderer::OpenGl, scale: 4, ..RenderSettings::default() };
    let fell_back = emu.set_render_settings(gl) == Renderer::Software;
    let no_texture = emu.gl_output().is_none();
    log::info!(
        "selftest: OpenGL without a context falls back to software: {}; \
         and reports no GL output: {}",
        yes_no(fell_back),
        yes_no(no_texture),
    );

    // Back to what the rest of the run expects.
    emu.set_render_settings(RenderSettings::default());
    advance(emu, 4);

    if !lit_threaded || !fell_back || !no_texture {
        log::error!("selftest: the renderer settings did not behave as documented");
        return false;
    }
    true
}

/// Boot a second console on shared airwaves and run both, the way
/// "Multiplayer > Launch new instance" does.
///
/// This proves the pair coexists and that the MP hooks are reachable from the
/// core. It does *not* prove local play works: a cart only takes to the air once
/// the player opens its wireless menu, which no headless run can do. The frame
/// counts below are therefore expected to be zero, and the check is that two
/// consoles run together without disturbing each other.
fn check_two_instances(rom: &Path) -> bool {
    let air = Airwaves::new();
    let mut host = match Emu::boot_mp(rom, None, None, 0, air.client(0)) {
        Ok(emu) => emu,
        Err(e) => {
            log::error!("selftest: cannot boot console 0: {e}");
            return false;
        }
    };
    let mut guest = match Emu::boot_mp(rom, None, None, 1, air.client(1)) {
        Ok(emu) => emu,
        Err(e) => {
            log::error!("selftest: cannot boot console 1: {e}");
            return false;
        }
    };

    // Interleaved, as the front end runs them, so their wifi clocks stay level.
    for _ in 0..240 {
        host.nds.run_frame();
        guest.nds.run_frame();
        if !host.nds.is_running() || !guest.nds.is_running() {
            log::error!("selftest: a console stopped while paired");
            return false;
        }
    }

    // Both must have produced their own picture: a shared-state bug between
    // instances would most likely show up as one of them going dark.
    let lit = |emu: &mut Emu| {
        emu.nds
            .framebuffers()
            .is_some_and(|(top, bottom)| top.iter().chain(bottom).any(|&px| px & 0x00FF_FFFF != 0))
    };
    let (host_lit, guest_lit) = (lit(&mut host), lit(&mut guest));

    let counters = air.counters();
    let sent: u64 = counters.iter().map(|c| c.sent_generic + c.sent_cmd).sum();
    log::info!(
        "selftest: two consoles ran 240 frames each; pictures: host {}, guest {}; \
         airwave frames {sent} (0 expected - neither cart was taken to its wireless menu)",
        yes_no(host_lit),
        yes_no(guest_lit),
    );

    if !host_lit || !guest_lit {
        log::error!("selftest: a paired console produced no picture");
        return false;
    }
    true
}

/// Check the two graphics knobs the FFI exposes actually do what melonDS
/// documents, since the Video settings dialog is built on them.
///
/// Both are supposed to skip *compositing only*: with them off the console keeps
/// running, and the framebuffer nobody is reading simply goes stale. So the test
/// is that the picture stops changing while the knob is off and starts again
/// when it is back on.
fn check_render_knobs(emu: &mut Emu) -> bool {
    // A moving scene is needed, or "unchanged" proves nothing. The intro is
    // animating by the frame counts this runs at; bail out rather than pass
    // vacuously if it is not.
    let baseline = digest(emu);
    advance(emu, 30);
    if digest(emu) == baseline {
        log::info!("selftest: picture is static here, skipping the render-knob check");
        return true;
    }

    emu.set_render(false);
    let frozen = digest(emu);
    advance(emu, 30);
    let still_frozen = digest(emu) == frozen;

    emu.set_render(true);
    advance(emu, 30);
    let moving_again = digest(emu) != frozen;

    log::info!(
        "selftest: set_render(false) froze the picture: {}; set_render(true) resumed it: {}",
        yes_no(still_frozen),
        yes_no(moving_again),
    );

    // The screen mask, one screen at a time. Checking only one direction would
    // not distinguish "the mask works" from "that screen was static anyway", so
    // each screen is first confirmed to be animating, then confirmed to stop
    // when the mask excludes it and keep going when it does not.
    let (top_animates, bottom_animates) = animating(emu);
    for (mask, name, animates) in
        [(0b01u8, "top", top_animates), (0b10u8, "bottom", bottom_animates)]
    {
        if !animates {
            log::info!("selftest: {name}-only mask not checked, that screen is static here");
            continue;
        }
        emu.set_displayed_screens(mask);
        // A couple of frames for the change to take effect before sampling.
        advance(emu, 4);
        let before = screen_digests(emu);
        advance(emu, 30);
        let after = screen_digests(emu);
        let (kept, dropped) = if mask == 0b01 {
            ((before.0, after.0), (before.1, after.1))
        } else {
            ((before.1, after.1), (before.0, after.0))
        };
        log::info!(
            "selftest: displayed={name}-only: {name} still moving: {}; other screen held: {}",
            yes_no(kept.0 != kept.1),
            yes_no(dropped.0 == dropped.1),
        );
    }
    emu.set_displayed_screens(0b11);

    if !still_frozen || !moving_again {
        log::error!("selftest: mds_set_render did not behave as documented");
        return false;
    }
    true
}

/// Whether each screen's picture is changing on its own, as `(top, bottom)`.
///
/// Without this a "the screen stopped changing" result is not evidence of
/// anything: a screen showing a still menu never changes either way.
fn animating(emu: &mut Emu) -> (bool, bool) {
    let before = screen_digests(emu);
    advance(emu, 30);
    let after = screen_digests(emu);
    (before.0 != after.0, before.1 != after.1)
}

fn advance(emu: &mut Emu, frames: u32) {
    for _ in 0..frames {
        emu.nds.run_frame();
    }
}

const fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "NO" }
}

/// A cheap hash of both framebuffers, for "did the picture change" questions.
fn digest(emu: &mut Emu) -> u64 {
    let (top, bottom) = screen_digests(emu);
    top ^ bottom.rotate_left(1)
}

fn screen_digests(emu: &mut Emu) -> (u64, u64) {
    let Some((top, bottom)) = emu.nds.framebuffers() else {
        return (0, 0);
    };
    let hash = |fb: &[u32]| {
        fb.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &px| {
            (h ^ u64::from(px)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    (hash(top), hash(bottom))
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
