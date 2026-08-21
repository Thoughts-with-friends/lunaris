# 20. Cheats, Debug Tools and Frontends

The last chapter covers everything that is not hardware: the Action Replay
interpreter, the diagnostic instrumentation woven through the core, the
built-in debug viewers, and the boundary between `nds-core` and the GUI.

---

## 20.1 Action Replay codes

An AR code is a little bytecode program, executed once per frame after
`emulate_frame` returns (Chapter 1, §1.3):

```rust
    if self.hw.enable_cheats {
        self.hw.apply_cheats();
    }
```

```rust
pub type CheatMap = Vec<ArCode>;

/// A single Action Replay style cheat code.
///
/// `code` holds the raw instruction stream, interpreted two `u32` words
/// (opcode+address, parameter) at a time, exactly as the original AR VM does.
pub struct ArCode {
    pub code: Vec<u32>,
    pub enabled: bool,
}
```

([lib.rs:42-51](core/src/lib.rs#L42-L51))

```text
   an AR code as the user types it        as the VM sees it
   ────────────────────────────────       ─────────────────
   0223DD34 60080180                      pair 0: hi=0x0223DD34 lo=0x60080180
   0223DD38 309C1C28                      pair 1: hi=0x0223DD38 lo=0x309C1C28
                │        │
                │        └── parameter
                └─────────── opcode (top nibble) + address (rest)
```

The interpreter is a close port of DeSmuME's `CHEATS::ARparser`, with one
deliberate improvement ([ar.rs:1-10](core/src/hw/ar.rs#L1-L10)):

```rust
// A close port of desmume's `CHEATS::ARparser` (desmume/src/cheatSystem.cpp),
// now routed through the full ARM7/ARM9 address space (via `HW::arm7_*` /
// `HW::arm9_*`) instead of a main-RAM-only buffer, so that codes which poll
// I/O registers (KEYINPUT, etc.) work the same way they do in desmume.
```

That routing choice is what makes conditional codes work
([ar.rs:11-19](core/src/hw/ar.rs#L11-L19), [ar.rs:52-56](core/src/hw/ar.rs#L52-L56)):

```rust
/// Which CPU's address space a cheat instruction currently targets.
/// Mirrors desmume's `st.proc` (`ARMCPU_ARM7` / `ARMCPU_ARM9`), which
/// defaults to ARM7 and can be switched at runtime via the `0xDF` opcode.
enum ArProc {
    Arm7,
    Arm9,
}
```

```rust
        // desmume: `st.proc = ARMCPU_ARM7;` -- AR codes default to targeting
        // the ARM7 bus (this is why KEYINPUT-based "hold R+B" codes work:
        // 0x04000130 lives in ARM7/ARM9-shared I/O space).
        let mut proc = ArProc::Arm7;
```

```text
   The VM's state, mirroring desmume's `st` struct
   ┌────────────────────────────────────────────────────────┐
   │ status           conditional-skip nesting bits         │
   │ offset           base address for indirect writes      │
   │ data             scratch register                      │
   │ proc             ARM7 / ARM9 target bus                │
   │ loop_status                                            │
   │ loop_idx         ┐                                     │
   │ loop_iterations  ├ the FOR/NEXT construct              │
   │ loop_top         ┘                                     │
   └────────────────────────────────────────────────────────┘

   opcode families (top nibble of hi)
   0/1/2  write 32/16/8 bits
   3..7   conditionals: if [addr] </>/=/!= value, else skip
   8..B   more conditionals and offset arithmetic
   C      FOR loop
   D      control: NEXT, offset ops, 0xDF = switch CPU
   E      block copy
   F      memory fill
```

Being a _close_ port rather than a fresh implementation matters here: AR codes
in the wild were written and tested against DeSmuME's exact quirks, so
faithfulness to the reference interpreter is worth more than
specification-correctness.

---

## 20.2 The diagnostics system

Emulator bugs are frequently invisible: the screen is black, or the room never
appears, and nothing has "failed". Lunaris addresses that with **opt-in probes
compiled into the release build** ([diag.rs:1-17](core/src/hw/diag.rs#L1-L17)):

```rust
//! Opt-in runtime diagnostics for rendering / audio triage.
//!
//! Every probe is gated on the `LUNARIS_DIAG` environment variable, which
//! holds a comma-separated list of probe names (or `all`). Nothing is printed
//! and no per-frame work is done unless a probe is requested, so release runs
//! are unaffected.
```

| Name      | Contents                                           |
| --------- | -------------------------------------------------- |
| `dispcnt` | D-1: per-frame DISPCNT + BGCNT for both 2D engines |
| `layers`  | D-2: per-BG opacity of one representative scanline |
| `vramcnt` | D-3: every VRAMCNT write (bank, MST, OFS, enable)  |
| `mosaic`  | D-4: every MOSAIC write (BG and OBJ sizes)         |
| `spu`     | D-5: SPU channel / SOUNDCNT writes                 |
| `capture` | D-6: DISPCAPCNT state at the start of each capture |
| `mix`     | peak mixed audio amplitude, once a second          |

The macro evaluates its arguments **only** when the probe is on
([diag.rs:39-48](core/src/hw/diag.rs#L39-L48)):

```rust
#[macro_export]
macro_rules! diag {
    ($name:literal, $($arg:tt)*) => {
        if $crate::hw::diag::probe($name) {
            eprintln!("[diag:{}] {}", $name, format_args!($($arg)*));
        }
    };
}
```

and `probe` caches the parsed environment in a `OnceLock`
([diag.rs:21-37](core/src/hw/diag.rs#L21-L37)) so a per-scanline call costs a
slice scan over a usually-empty `Vec`.

```text
   $ LUNARIS_DIAG=dispcnt,layers cargo run --release
   [diag:dispcnt] engineA mode=1 bg0=3d bg1=on prio=1 ...
   [diag:layers]  engineA bg1: enabled=1 prio=1 opaque=256/256 first=0x8421
   [diag:layers]  engineA bg2: enabled=1 prio=2 opaque=0/256   first=0x0000
                                                        ^^^^^
                            "bg2 is enabled but produced nothing"
                            — a VRAM banking or tile-base problem,
                              not a compositing one
```

Sibling variables, each answering one question:

```text
   LUNARIS_DIAG=<probes>   rendering / audio triage (above)
   LUNARIS_WIFI_DEBUG=1    TX/RX/beacon trace                (Chapter 18)
   LUNARIS_MP_DIAG         MP handshake counters             (Chapter 18)
   LUNARIS_SPI_TRACE       IR/SPI selector bytes             (Chapter 15)
   LUNARIS_MP_ASSOC_TRACE  assoc-response readback trace     (Chapter 18)
   LUNARIS_NO_IR           bypass the IR wrapper             (Chapter 15)
```

Note the pattern: each one is a **bisection tool**, not a log level. It splits
a failure space in half rather than printing more of everything.

---

## 20.3 The debug viewers

[gpu/debug.rs](core/src/hw/gpu/debug.rs) renders emulator state _as images_,
using the same decode paths the real renderer uses.

```text
   Palettes            Tiles                Maps                VRAM
   ┌──────────────┐    ┌──────────────┐     ┌──────────────┐    ┌────────┐
   │▓▓▓▒▒▒░░░ ...│    │ ▞▚▞▚ ▞▚▞▚   │     │ the whole    │    │ raw    │
   │ 16×16 grid   │    │ every tile   │     │ tilemap as   │    │ bank   │
   │ of 8×8 swatch│    │ in a char    │     │ one image    │    │ bytes  │
   │              │    │ block        │     │              │    │        │
   └──────────────┘    └──────────────┘     └──────────────┘    └────────┘
```

```rust
    pub fn render_palettes<F: Fn(usize) -> u16>(
        get_color: F,
        palettes_size: usize,
    ) -> (Vec<u16>, usize, usize) {
        let size = palettes_size * 8;
        let mut pixels = vec![0; size * size];
        for palette_y in 0..palettes_size {
            for palette_x in 0..palettes_size {
                let color_num = palette_y * palettes_size + palette_x;
                let start_i = (palette_y * size + palette_x) * 8;
```

([gpu/debug.rs:4-22](core/src/hw/gpu/debug.rs#L4-L22))

Each returns `(Vec<u16>, width, height)` — the same BGR555 format as a real
screen, so the frontend uploads it through exactly the same path. That is the
design decision worth copying: a debug view that shares the production decode
path cannot disagree with it.

The core exposes them as plain methods
([nds.rs:283-291](core/src/nds.rs#L283-L291)):

```rust
    pub fn render_palettes(
        &self,
        extended: bool,
        slot: usize,
        palette: usize,
        engine: Engine,
        graphics_type: GraphicsType,
    ) -> (Vec<u16>, usize, usize) {
```

---

## 20.4 The core/frontend boundary

```text
   ┌───────────────────────── nds-core ─────────────────────────┐
   │  in:   press_key / release_key / press_screen              │
   │        set_cheat_map / set_enable_cheats                   │
   │        set_audio_volume / set_audio_sync                   │
   │        set_mp_transport / set_wifi_clock_epoch             │
   │        import_save / load_state                            │
   │                                                            │
   │  do:   emulate_frame()                                     │
   │                                                            │
   │  out:  get_screens() -> [&Vec<u16>; 2]                     │
   │        save_state / export_save / rom_fingerprint          │
   │        render_palettes / render_tiles / render_map         │
   │        wifi_diag_snapshot / wifi_link_hints                │
   └────────────────────────────────────────────────────────────┘

   NOT in the core: windows, file dialogs, config files, sockets,
                    audio device enumeration, input mapping, upscaling
```

`nds-core` has no `main`, no `std::net`, and no UI dependency. Everything on
the other side of that line lives in `gui/`.

### The three frontends

```text
   gui/egui        default. eframe/egui, multi-viewport, debug windows,
                   cheat editor, input rebinding, LAN room UI.

   gui/imgui       the original Dear ImGui frontend, kept working.

   gui/melon_egui  the same egui shell driving the REAL melonDS core through
                   melonds-rs. Not a Lunaris frontend at all — a controlled
                   comparison. When a game misbehaves, running it here answers
                   "is this our bug or the game's?" in one step.

   gui/common      what the first two share: config, savestate container,
                   framebuffer conversion, upscaling, input mapping,
                   ROM loading, logging.
   gui/net         LAN rooms, discovery, netplay transport.
```

Keeping a reference frontend around is unusual and worth calling out — it
converts "does this look right to me?" into "does this match melonDS?", which
is a question with an answer.

### Frontend-side features

```text
   framebuffer.rs   BGR555 → RGBA8, the one format conversion
   upscale.rs       xBRZ post-process upscaling of the finished RGBA8 frame
   savestate.rs     the LNST container (Chapter 19)
   cheat_map.rs     .mch / AR code file parsing
   config.rs        persisted settings
   input/           keyboard + gamepad mapping, rebindable
   fonts.rs         CJK font fallback, so Japanese cheat names render
```

The upscaler's doc comment states the boundary explicitly
([gui/common/src/upscale.rs:1-7](gui/common/src/upscale.rs#L1-L7)):

```rust
//! Post-process upscaling of the finished RGBA8 screen buffers.
//!
//! Deliberately operates on the already-composited RGBA8 frame produced by
//! [`crate::framebuffer::abgr1555_to_rgba8`], not on any `core` state — see
//! `docs/design/resolution-upscaling-design.md` §2 for why raising the
//! *internal* NDS resolution is out of scope and why post-processing the
//! finished frame is the safe place for this feature.
```

Raising the internal resolution would mean touching the rasteriser, the
framebuffer, display capture, and every VRAM offset. Upscaling the finished
frame touches one function. When a feature can be implemented at the boundary,
implement it at the boundary.

---

## 20.5 Building and running

```text
   cargo run                           → gui/egui
   cargo xtask run  --gui egui         → same, explicit
   cargo xtask run  --gui imgui        → Dear ImGui frontend
   cargo xtask build --gui melon       → melonDS reference build
                                          (auto-locates an LLVM toolchain)

   cargo test -p nds-core              → core unit tests
   cargo run -p nds-core --example dump_frame   → headless frame dump
   cargo run -p nds-core --example mp_loopback  → local MP smoke test
```

The examples matter more than they look: `dump_frame` renders a frame headless
so a rendering regression can be diffed in CI, and `mp_loopback` drives the
Wi-Fi hardware directly through the `wifi_write16` escape hatch
([nds.rs:114-122](core/src/nds.rs#L114-L122)) without needing a Wi-Fi-capable
test ROM.

---

## 20.6 Where to go next

```text
   want to...                        read
   ───────────────────────────────   ────────────────────────────────────
   write your own DS emulator        Chapter 1, then 3 → 5 → 6 → 7
   understand a black screen         Chapters 9, 12, and §20.2
   understand a freeze               Chapters 10 (§10.3), 19 (§19.4)
   understand a save problem         Chapter 15
   work on multiplayer               Chapter 18, docs/design/local_mp/
   compare against real hardware     GBATEK, linked per chapter
   compare against another emulator  docs/design/melonds/ (vendored)
```

Design documents for individual investigations live in
[docs/design/](docs/design/); the completed ones are under
`docs/design/complete/`. Each one records not only what was fixed but what the
symptom looked like — which, for an emulator, is usually the harder half.

---

[← 19. Savestates](19_savestates.md) | [Back to Chapter 1 ↩](01_emulator_architecture.md)
