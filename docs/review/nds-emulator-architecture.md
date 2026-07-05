# Lunaris NDS Emulator — Detailed Design Specification

This document describes how the `nds-core` crate implements the Nintendo DS
hardware specification, block by block, and how the pieces interact over
time: from loading a ROM path, through initialization and direct boot, to
the steady-state frame loop driven by scheduled hardware events.

Every section links to the corresponding GBATEK chapter
(<https://problemkaputt.de/gbatek.htm>), which is the authoritative hardware
reference this implementation follows. The same pinpoint links are embedded
as Rust doc comments in the source files listed per section.

---

## 1. System Overview

```
                 ┌────────────────────────────────────────────────┐
                 │                    NDS (nds.rs)                │
                 │  ┌───────────┐  ┌───────────┐  ┌────────────┐  │
                 │  │ ARM9      │  │ ARM7      │  │    HW      │  │
                 │  │ ARM946E-S │  │ ARM7TDMI  │  │ (hw.rs)    │  │
                 │  │ 2× clock  │  │ 1× clock  │  │            │  │
                 │  └─────┬─────┘  └─────┬─────┘  └─────┬──────┘  │
                 └────────┼──────────────┼──────────────┼─────────┘
                          │ arm9_read/   │ arm7_read/   │
                          │ arm9_write   │ arm7_write   │
              ┌───────────┴──────────────┴──────────────┴───────────┐
              │  Scheduler ── GPU (2D A/B + 3D) ── SPU ── DMA ×8    │
              │  Timers ×8 ── IPC ── IRQ ctrl ×2 ── Cartridge       │
              │  SPI (firmware / TSC) ── RTC ── Keypad ── Div/Sqrt  │
              │  Memory: main RAM, WRAM, VRAM A-I, ITCM/DTCM, BIOS  │
              └─────────────────────────────────────────────────────┘
```

* `NDS` ([core/src/nds.rs](../../core/src/nds.rs)) owns both CPU cores and
  the shared hardware aggregate `HW`.
* `HW` ([core/src/hw.rs](../../core/src/hw.rs)) owns every peripheral plus
  the cycle-accurate event `Scheduler`.
* All inter-block timing flows through the scheduler; nothing polls.

GBATEK: [DS Technical Data](https://problemkaputt.de/gbatek.htm#dstechnicaldata)

### 1.1 Clock model

| Clock                   | Value                                   | Where defined                                    |
| ----------------------- | --------------------------------------- | ------------------------------------------------ |
| Master (bus/ARM7) clock | 33.513982 MHz (`NDS::CLOCK_RATE`)       | `nds.rs`                                         |
| ARM9 clock              | 2 × master (66.028 MHz)                 | implicit: ARM9 cycle counter runs at `2 × cycle` |
| Dot clock               | master / 6 = 5.585664 MHz               | `GPU::CYCLES_PER_DOT`                            |
| Sound sample clock      | host device rate (hardware: 32.768 kHz) | `SPU::clocks_per_sample`                         |

The scheduler's `cycle` counter is in **master-clock units**. The ARM9 keeps
its own counter in half-cycles: when the frame loop targets master cycle
`T`, ARM9 runs until `2 T` and ARM7 until `T`.

---

## 2. Initialization Sequence (`NDS::load_rom` → first frame)

Entry point: `NDS::load_rom(bios7_path, bios9_path, firmware_path, rom_path,
audio_volume)` in [core/src/nds.rs](../../core/src/nds.rs).

1. **File resolution**
   - `rom_path.sav` is opened/created as the backup file (never truncated —
     truncation corrupts existing saves).
   - BIOS7/BIOS9 fall back to the bundled `free_bios` images when no path
     is given.
   - Firmware falls back to a FreeBIOS firmware image materialized in the
     OS temp directory.

2. **`HW::new` construction order** (matters, because peripherals schedule
   their first events during construction):
   1. `Scheduler::new()` — empty priority queue, `cycle = 0`.
   2. `Cartridge::new(rom, save_file, &bios7)` — parses the 200h-byte header
      ([GBATEK: DS Cartridge Header](https://problemkaputt.de/gbatek.htm#dscartridgeheader)),
      initializes the KEY1 Blowfish buffer from BIOS7 offset 30h
      ([GBATEK: KEY1](https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1)),
      and selects the backup chip by game-code lookup
      ([GBATEK: DS Cartridge Backup](https://problemkaputt.de/gbatek.htm#dscartridgebackup)).
   3. `GPU::new(&mut scheduler)` — **schedules the first `HBlank` event** at
      dot 264 (`264 × 6` master cycles).
   4. `SPU::new(&mut scheduler)` — **schedules the first
      `GenerateAudioSample` event** at `CLOCK_RATE / host_sample_rate`
      cycles.
   5. `SPI::new(firmware_file)` — memory-maps the firmware and patches the
      user-settings touch calibration + CRC16
      ([GBATEK: DS Firmware User Settings](https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings)).
   6. Page tables for both CPUs are built (§7.2).

3. **Direct boot** (`direct_boot == true`, the only supported path today):
   - `HW::init_mem` mirrors what the BIOS does on a retail cold boot:
     header copied to `27FFE00h`, chip ID to `27FF800h`/`27FFC00h`, plus
     boot status words
     ([GBATEK: header load address](https://problemkaputt.de/gbatek.htm#dscartridgeheader)).
   - `ARM::new(hw, direct_boot)` loads the ARM9/ARM7 code segments described
     by the header into RAM, points PC at the header entry addresses, and
     initializes SP/CPSR the way the firmware would.
   - `POSTFLG7/9` are set to 1 so games see "boot completed".
   - In non-direct boot, `Cartridge::encrypt_secure_area` instead re-encrypts
     the secure area so the (Free)BIOS boot path can decrypt it
     ([GBATEK: DS Cartridge Secure Area](https://problemkaputt.de/gbatek.htm#dscartridgesecurearea)).

After construction, the scheduler contains exactly two pending events —
`HBlank` and `GenerateAudioSample` — and both CPUs are ready at cycle 0.

---

## 3. Steady-State Frame Loop

`NDS::emulate_frame` runs until the GPU reports a completed frame:

```text
while !hw.rendered_frame():
    if 3D bus not stalled:
        target = min(current_cycle + 30, cycle_of_next_event)
        arm9.emulate(hw, target * 2)     # ARM9 in half-cycles
        arm7.emulate(hw, target)
        hw.clock_until(target)           # fire due events
    else:
        hw.clock_until_event()           # advance time by events only
        resync both CPU cycle counters
```

Design decisions:

* **30-cycle slices** bound the desync between ARM9 and ARM7. Both CPUs run
  ahead independently within a slice; hardware events are only processed at
  slice boundaries. 30 cycles was measured as the worst observed desync
  when the CPUs ran tightly coupled.
* **Event horizon**: a slice never crosses `cycle_at_next_event()`, so an
  event can never fire "in the past" relative to either CPU.
* **3D bus stall** (`Engine3D::bus_stalled`): when the geometry FIFO holds
  ≥ 256 entries, real hardware blocks further CPU writes
  ([GBATEK: GXFIFO overflow](https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands)).
  The emulator models this by freezing both CPUs and fast-forwarding the
  scheduler event-by-event until the FIFO drains (which happens in the
  VBlank handler via `exec_commands`). After each event both CPU counters
  are re-synced to the scheduler clock.
* Each CPU checks IRQs (`handle_irq`) and halt state before every
  instruction; a halted CPU consumes its whole slice at once.

### 3.1 Scheduler

[core/src/hw/scheduler.rs](../../core/src/hw/scheduler.rs) — a min-heap of
`(Event, fire_cycle)` with function-pointer handlers.

| Event                        | Producer                      | Handler / consumer                        |
| ---------------------------- | ----------------------------- | ----------------------------------------- |
| `HBlank`                     | GPU (self-rescheduling)       | `HW::on_hblank`                           |
| `StartNextLine`              | `on_hblank`                   | `HW::start_next_line`                     |
| `VBlank`                     | `start_next_line` at line 192 | `HW::on_vblank` (called directly)         |
| `DMA(is_nds9, ch)`           | DMA enable writes, triggers   | `HW::on_dma`                              |
| `CheckGeometryCommandFIFO`   | DMA writes, GXFIFO writes     | `HW::check_geometry_command_fifo_handler` |
| `TimerOverflow(is_nds9, i)`  | `Timer::create_event`         | `HW::on_timer_overflow`                   |
| `ROMWordTransfered(is_nds9)` | `Cartridge::run_command`      | `HW::on_rom_word_transfered`              |
| `ROMBlockEnded(is_nds9)`     | cartridge word stream end     | `HW::on_rom_block_ended`                  |
| `GenerateAudioSample`        | SPU (self-rescheduling)       | `HW::generate_audio_sample`               |
| `StepAudioChannel(spec)`     | channel start / timer writes  | `HW::step_audio_channel`                  |
| `ResetAudioChannel(spec)`    | one-shot sample end           | `HW::reset_audio_channel`                 |

Savestate note: function pointers cannot be serialized, so the queue is
flattened to `(event_types[], fire_cycles[])` on save and rebuilt through
`Scheduler::restore_events` + `HW::handler_for_event` on load. All absolute
cycle counters are serialized as `u64` (see
[savestate-and-video-design.md](savestate-and-video-design.md) §3 for the
`usize`→`u32` truncation bug this prevents).

---

## 4. Video Pipeline

### 4.1 Timing skeleton

GBATEK: [DS Video Stuff — Display Dimensions/Timings](https://problemkaputt.de/gbatek.htm#dsvideostuff)

```
1 dot        = 6 master cycles (dot clock 5.585664 MHz)
1 scanline   = 355 dots  = 2130 cycles   (256 visible + 99 blank)
1 frame      = 263 lines = 560,190 cycles ≈ 59.83 Hz
visible area = 256 × 192, lines 0..191; VBlank = lines 192..262
HBlank start = dot 264 (256 + 8-dot delay)
```

Per scanline, two events alternate:

1. **`HBlank` (dot 264)** — `HW::on_hblank`
   - Sets the HBLANK flag in both CPUs' DISPSTAT; raises H-Blank IRQ if
     enabled ([GBATEK: DISPSTAT](https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus)).
   - On visible lines: renders the line (Engine A, then Engine B, then
     display capture) and starts H-Blank DMA.
   - Schedules `StartNextLine` for dot 355 (= dot 0 of the next line).

2. **`StartNextLine` (dot 0)** — `HW::start_next_line`
   - Clears HBLANK flags, increments VCOUNT (wraps at 263).
   - Line 262→0 boundary re-latches the affine reference points
     ([GBATEK: BG rotation/scaling internal registers](https://problemkaputt.de/gbatek.htm#lcdiobgrotationscaling)).
   - Line 0: clears VBLANK flags, latches `DISPCAPCNT.enable` for the frame.
   - Line 192: sets VBLANK flags, marks the frame as rendered, raises the
     V-Blank IRQ, and calls `on_vblank` (V-Blank DMA + 3D rendering, §5).
   - Every line: compares VCOUNT with each DISPSTAT's setting and raises
     the V-counter-match IRQ.

The `rendered_frame` flag set at line 192 is what terminates
`NDS::emulate_frame`; the GUI then presents both engines' `pixels` buffers.

### 4.2 2D engines (A and B)

[core/src/hw/gpu/engine2d.rs](../../core/src/hw/gpu/engine2d.rs) — one
generic `Engine2D<EngineA|EngineB>` instance each; Engine A additionally
supports the 3D layer on BG0 and display capture.
GBATEK: [DS Video](https://problemkaputt.de/gbatek.htm#dsvideo),
[BG Modes/Control](https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol).

Scanline rendering is **line-buffer based**, mirroring hardware:

1. `render_line` dispatches on the DISPCNT *display mode*:
   off/white (0), normal (1), VRAM bitmap a.k.a. LCDC mode (2), main-memory
   FIFO (3, unimplemented).
2. Normal mode (`render_normal_line`):
   - Window 0/1 per-dot masks (`render_window`), OBJ line
     (`render_objs_line`), then BG0..BG3 according to the *BG mode* table
     (text / affine / extended per mode 0-5; mode 6 unimplemented).
   - Engine A BG0 becomes the 3D frame-buffer line when DISPCNT bit 3 is
     set (`render_bg0` → `Engine3D::copy_line`).
3. `process_lines` composites the 4 BG line buffers + OBJ line into final
   pixels: priority sort, window gating, then color special effects
   (alpha blend / brighten / darken via BLDCNT
   ([GBATEK](https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects)))
   and OBJ semi-transparency.

Palette/OAM storage lives inside each engine (`bg_palettes`,
`obj_palettes`, `oam`); extended palettes are VRAM-mapped and fetched
through `VRAM::get_bg_ext_pal` / `get_obj_ext_pal`
([GBATEK: DS Video Extended Palettes](https://problemkaputt.de/gbatek.htm#dsvideoextendedpalettes)).

### 4.3 3D engine

[core/src/hw/gpu/engine3d.rs](../../core/src/hw/gpu/engine3d.rs) splits,
like hardware, into a **geometry engine** and a **rendering engine**
([GBATEK: DS 3D Overview](https://problemkaputt.de/gbatek.htm#ds3doverview)).

Geometry (`geometry.rs`):
- Commands enter via GXFIFO packed-format writes (4000400h) or per-command
  mirror ports (4000440h+)
  ([GBATEK: Geometry Commands](https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands)).
- `exec_commands` drains the FIFO **eagerly** (not cycle-timed) and stops at
  `SwapBuffers`, setting `polygons_submitted` — hardware likewise halts the
  geometry engine until the VBlank buffer swap.
- Matrix stacks (projection ×1, position/vector ×31, texture ×1), lighting
  (4 lights, material colors), texture-coordinate transform, and clipping
  against the view volume are performed at vertex-submission time.
- FIFO-level flow control: `bus_stalled` at ≥256 entries (§3); DMA start
  mode 7 refills when below half
  ([GBATEK: DMA start modes](https://problemkaputt.de/gbatek.htm#dsdmatransfers));
  GXSTAT IRQ conditions (never/less-half/empty)
  ([GBATEK: GXSTAT](https://problemkaputt.de/gbatek.htm#ds3dstatus)).

Rendering (`rendering.rs`):
- Runs once per frame **inside `on_vblank`**, only when POWCNT1 bit 2 is
  set: clears the frame buffer to CLEAR_COLOR/CLEAR_DEPTH, then
  scan-converts each polygon with perspective-correct interpolation,
  depth-test, texture sampling (all 7 formats
  ([GBATEK](https://problemkaputt.de/gbatek.htm#ds3dtextureformats))), and
  modulation/decal/toon blending
  ([GBATEK](https://problemkaputt.de/gbatek.htm#ds3dtextureblending)).
- Hardware actually renders scanline-by-scanline ~48 lines ahead of the
  display; rendering the whole frame at VBlank is an accepted
  simplification (documented divergence).
- After rendering, `exec_commands` resumes buffered commands and
  `check_geometry_command_fifo` re-arms GXFIFO DMA.

### 4.4 VRAM banking

[core/src/hw/gpu/vram.rs](../../core/src/hw/gpu/vram.rs) implements the 9
banks (A-I, 656 KiB total) with dynamic MST/OFS mapping into LCDC space,
engine BG/OBJ areas, extended palettes, 3D texture/palette slots, and ARM7
WRAM — a direct transcription of the mapping tables in
[GBATEK: DS Memory Control — VRAM](https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram).
Each function keeps a per-region `Vec<Vec<Bank>>` overlay list so
overlapping banks OR together like the real bus.

### 4.5 Display capture

`GPU::capture` (called during `render_line` for lines inside the capture
size) implements DISPCAPCNT: source A = Engine A composite or raw 3D layer,
source B = VRAM block, blended with EVA/EVB, written to the selected VRAM
bank/offset. `enable` auto-clears at the end of the captured frame.
GBATEK: [DS Video Capture](https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode).

---

## 5. Audio Pipeline (SPU) and Its Clock Coupling

[core/src/hw/spu.rs](../../core/src/hw/spu.rs).
GBATEK: [DS Sound](https://problemkaputt.de/gbatek.htm#dssound),
[Channels 0..15](https://problemkaputt.de/gbatek.htm#dssoundchannels015),
[Control Registers](https://problemkaputt.de/gbatek.htm#dssoundcontrolregisters).

Two independent event cadences drive audio:

1. **Per-channel sample stepping** — `Event::StepAudioChannel(spec)`.
   Writing SOUNDxCNT busy=1 schedules the channel with period
   `(10000h − timer_val) × 2` master cycles, exactly the hardware timer
   formula (the ×2 is because the sound timer counts at the ARM9-side
   2× clock). Each firing fetches the next PCM8/PCM16 sample or ADPCM
   nibble **through the ARM7 bus** (`arm7_read`), so sample data honors
   WRAM/VRAM-C/D mapping. End-of-data behavior follows the repeat mode:
   manual, loop (rewind to `loop_start`), or one-shot
   (schedule `ResetAudioChannel`, which zeroes the sample and clears busy).
   - Channels 0-7: PCM/ADPCM only; 8-13: + PSG square duty; 14-15: + noise
     LFSR.
   - ADPCM uses the IMA tables and the exact GBATEK decode pseudo-code
     ([DS Sound Notes](https://problemkaputt.de/gbatek.htm#dssoundnotes)).

2. **Mixer output** — `Event::GenerateAudioSample`, self-rescheduled every
   `clocks_per_sample = CLOCK_RATE / host_rate` cycles. It sums all
   channels with per-channel volume shift/factor and 128-step panning,
   applies SOUNDCNT's left/right source select (mixer, ch1, ch3, ch1+ch3)
   and master volume, and pushes one stereo `i16` pair to the `cpal` ring
   buffer.
   - **Divergence**: hardware mixes at a fixed 32.768 kHz (1024 cycles of
     the 33.554 MHz sound clock) and the DAC applies SOUNDBIAS; the
     emulator instead samples natively at the host device rate (typically
     48 kHz) — a TODO in `SPU::new` tracks moving to native-rate +
     resampler.
   - Sound capture units 0/1 can read the mixer back into ARM7 memory
     ([GBATEK: DS Sound Capture](https://problemkaputt.de/gbatek.htm#dssoundcapture));
     channel-add and bugged modes are still `todo!()`.

Because both cadences are scheduler events measured in master-clock cycles,
audio stays phase-locked to video and CPU time by construction: a frame is
560,190 cycles, so at 48 kHz the mixer fires ~698 times per frame,
interleaved deterministically with H-Blank/DMA/timer events.

---

## 6. DMA, Timers, IPC, Interrupts

### 6.1 DMA (8 channels)

[core/src/hw/dma.rs](../../core/src/hw/dma.rs).
GBATEK: [DS DMA Transfers](https://problemkaputt.de/gbatek.htm#dsdmatransfers).

- Channels are indexed by start trigger (`Occasion`) in a `by_type` table
  so event handlers dispatch in O(active-channels).
- Trigger paths: `Immediate` runs via `run_now`; `HBlank` from
  `on_hblank` (visible lines only); `VBlank` from `on_vblank`;
  `DSCartridge` from the cartridge word-ready path;
  `GeometryCommandFIFO` whenever GXFIFO drops below half.
- A transfer runs **to completion within one event** (block transfer),
  reading and writing through the owning CPU's full bus dispatch, honoring
  ARM9 (21-bit) vs ARM7 (14/16-bit) word-count limits and per-channel
  address masks. Cycle costing is computed but currently not applied to
  the CPUs (documented TODO — loose CPU synchronization made it unsound).
- Completion raises `DMA0..3` IRQs on both controllers when enabled.

### 6.2 Timers (2 × 4)

[core/src/hw/timers.rs](../../core/src/hw/timers.rs).
GBATEK: [DS Timers](https://problemkaputt.de/gbatek.htm#dstimers),
[GBA Timers](https://problemkaputt.de/gbatek.htm#gbatimers).

- Regular timers are **not ticked**: the counter is derived lazily from
  `(scheduler.cycle − start_cycle)` and the prescaler (1/64/256/1024), and
  a single `TimerOverflow` event is scheduled for the exact overflow cycle,
  including the 1-cycle start delay and prescaler phase alignment.
- Count-up (cascade) timers have no event; the previous timer's overflow
  handler clocks them, recursing up the chain.
- ARM9 timers count master cycles like ARM7 timers (both are specified at
  F = 33.513982 MHz — the ARM9's 2× clock does *not* speed up its timers).

### 6.3 IPC

[core/src/hw/ipc.rs](../../core/src/hw/ipc.rs).
GBATEK: [DS IPC](https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc).

- IPCSYNC: 4-bit cross-wired data + remote-IRQ pulse.
- IPCFIFO: two 16-word queues; empty/not-empty transition IRQs; error flag
  on overflow/underflow; empty reads return the last received word. All
  behaviors follow the GBATEK register description bit-for-bit.
- This is the primary control channel between game code (ARM9) and the
  ARM7 system services (audio commands, touch/RTC results), so FIFO IRQ
  correctness is load-bearing for most commercial games.

### 6.4 Interrupt controllers (×2)

[core/src/hw/interrupt_controller.rs](../../core/src/hw/interrupt_controller.rs).
GBATEK: [DS Interrupts](https://problemkaputt.de/gbatek.htm#dsinterrupts).

- Per-CPU IME/IE/IF with the NDS bit layout (bits 16-21 for IPC, cartridge,
  GXFIFO). IF write-1-to-clear.
- Peripherals set bits in `interrupts[cpu].request` directly from their
  event handlers; the CPU cores test `(IE & IF) != 0` before each
  instruction (`ARM::handle_irq`) and vector to `base + 18h`
  (ARM9 base relocatable via CP15).
- Halt: ARM7 via HALTCNT, ARM9 via CP15 wait-for-IRQ; a halted ARM9 wakes
  on any enabled pending IRQ regardless of IME (hardware quirk, modeled by
  `interrupts_requested(ignore_ime)`).

---

## 7. Memory System

### 7.1 Maps

[core/src/hw/mem.rs](../../core/src/hw/mem.rs) and `mem/arm7.rs`,
`mem/arm9.rs`, `mem/arm7/io.rs`, `mem/arm9/io.rs`.
GBATEK: [DS Memory Maps](https://problemkaputt.de/gbatek.htm#dsmemorymaps),
[DS I/O Maps](https://problemkaputt.de/gbatek.htm#dsiomaps).

Backing stores: 4 MiB main RAM, 64 KiB ARM7 IWRAM, 32 KiB shared WRAM
(WRAMCNT-banked, [GBATEK](https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram)),
32 KiB ITCM + 16 KiB DTCM (CP15-mapped,
[GBATEK](https://problemkaputt.de/gbatek.htm#dsmemorycontrolcacheandtcm)),
BIOS images, and the nine VRAM banks.

### 7.2 Page-table fast path

Each CPU has a raw-pointer page table (ARM9: 4 KiB pages, ARM7: 16 KiB
pages) covering directly-backed RAM regions. Aligned reads/writes hit the
pointer path; null entries (I/O, VRAM with engine mapping, TCM boundary
regions) fall back to a `match` on `MemoryRegion` that dispatches to the
byte-oriented `IORegister` trait implementations. Page tables are rebuilt
after WRAMCNT/CP15 TCM changes and after every savestate load.

### 7.3 Access timing

`arm9_get_access_time` / `arm7_get_access_time` model N/S access classes
per region ([GBATEK: DS Memory Timings](https://problemkaputt.de/gbatek.htm#dsmemorytimings));
the CPU cores accumulate these into their cycle counters on every fetch,
read, and write, which is what the frame loop's `target` comparison
consumes.

---

## 8. CPU Cores

[core/src/arm.rs](../../core/src/arm.rs) with `arm/arm.rs` (ARM ISA),
`arm/thumb.rs` (THUMB ISA), `arm/registers.rs` (banked register file).
GBATEK: [ARM CPU Overview](https://problemkaputt.de/gbatek.htm#armcpuoverview).

- One generic core `ARM<const IS_ARM9: bool>`; the flag selects ARM9-only
  instructions (CLZ, QADD/QSUB, long multiplies timing, CP15 MRC/MCR) and
  the 2× cycle accounting.
- Decoding via prebuilt LUTs: 4096-entry ARM table indexed by opcode bits
  [27:20]|[7:4], 256-entry THUMB table indexed by bits [15:8]; condition
  codes via a 256-entry `(NZCV<<4)|cond` table.
- Two-word prefetch buffer models the pipeline (branches refill it).
- IRQ entry per [ARM CPU Exceptions](https://problemkaputt.de/gbatek.htm#armcpuexceptions):
  mode switch to IRQ, LR fixup by state (ARM/THUMB), I-bit set, vector
  `base+18h`.
- The BIOS is executed natively (no HLE); SWIs run the real BIOS code.

---

## 9. Cartridge, SPI, RTC, Keypad, Maths

### 9.1 Cartridge slot

[core/src/hw/cartridge.rs](../../core/src/hw/cartridge.rs).
GBATEK: [DS Cartridge Protocol](https://problemkaputt.de/gbatek.htm#dscartridgeprotocol),
[I/O Ports](https://problemkaputt.de/gbatek.htm#dscartridgeioports).

Command flow (either CPU may own the slot via EXMEMCNT):

```
write ROMCMD[0..7] → write ROMCTRL bit31
  → run_command: decode B7h (read), B8h (chip ID), KEY1 set, …
  → schedule ROMWordTransfered every ~word time
      → word pushed to game_card_words; DSCartridge DMA triggered;
        ROMCTRL.data_word_ready set
  → after block_size bytes: ROMBlockEnded
      → busy cleared; GAME_CARD_TRANSFER_COMPLETION IRQ if AUXSPICNT enable
```

KEY1-encrypted commands (pre-boot protocol) are decrypted with the Blowfish
state seeded from BIOS7; the secure area handling is described in §2.3.
Backup access goes through AUXSPI to the EEPROM/Flash model, persisted via
memory-mapped `.sav` file.

### 9.2 ARM7 SPI bus

[core/src/hw/spi.rs](../../core/src/hw/spi.rs) — SPICNT device select:
power manager (stub), firmware flash (full read/write/status command set),
TSC touch controller ([tsc.rs](../../core/src/hw/spi/tsc.rs), 12-bit X/Y
conversions; microphone channel returns 0).
GBATEK: [SPI Bus](https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi),
[TSC](https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc).

Touch input path: GUI → `NDS::press_screen(x, y)` → sets `EXTKEYIN.PEN_DOWN`
(keypad) + latches TSC coordinates; games poll via SPI conversions.

### 9.3 RTC

[core/src/hw/rtc.rs](../../core/src/hw/rtc.rs) — bit-banged serial protocol
of the S-35180 (command `0110` + register select), date/time supplied from
the host clock in BCD.
GBATEK: [DS RTC](https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc).

### 9.4 Keypad

[core/src/hw/keypad.rs](../../core/src/hw/keypad.rs) — KEYINPUT/KEYCNT
(both CPUs) + EXTKEYIN (ARM7). The keypad IRQ condition (OR/AND mode) is
evaluated lazily inside `arm*_interrupts_requested`, i.e. checked every
instruction boundary rather than on key events.
GBATEK: [DS Keypad](https://problemkaputt.de/gbatek.htm#dskeypad).

### 9.5 Hardware maths

[core/src/hw/math.rs](../../core/src/hw/math.rs) — ARM9 divider and square
root with all documented edge cases (div-by-zero, `i64::MIN / −1`,
mode-3-equals-mode-1 quirk used by *Kingdom Hearts 358/2 Days*). Results
are computed instantly on parameter write; the busy-cycle latency (18/34
cycles) is not modeled.
GBATEK: [DS Maths](https://problemkaputt.de/gbatek.htm#dsmaths).

---

## 10. Savestates (summary)

Full design in [savestate-and-video-design.md](savestate-and-video-design.md).
Key invariants relevant to the architecture:

- Everything except reconstructible state (page tables, audio device,
  backup mmap, event handler pointers, LUTs) is serialized via the
  `emu_utils::Savestate` derive.
- `HW::post_load_hw` rebuilds: scheduler queue (`restore_events`), both
  page tables, and clears `bus_stalled` (a stale stall would deadlock the
  frame loop).
- All absolute cycle counters (`ARM::cycle`, `Scheduler::cycle`, timer
  `start_cycle`, event fire cycles) are stored as `u64` because the
  serializer truncates `usize` to `u32` (~64 s of ARM9 time).
- `Vec`/`VecDeque` fields use `Loadable` (not `load_in_place`) because
  in-place loading does not consume the length prefix.

---

## 11. Known Divergences from Hardware (accepted / TODO)

| Area            | Divergence                                                                                            | Impact                                                     |
| --------------- | ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| 3D rendering    | whole-frame at VBlank instead of scanline pipeline                                                    | capture/`copy_line` mid-frame effects off by up to a frame |
| Geometry engine | commands execute instantly (no per-command cycle cost)                                                | GXSTAT busy timing optimistic                              |
| SPU             | mixes at host rate, not 32.768 kHz; SOUNDBIAS unused; capture add/bugged modes `todo!()`              | minor fidelity; capture-heavy games may break              |
| DMA             | transfer cycles not charged to CPUs                                                                   | timing-sensitive races resolve too fast                    |
| Maths           | DIV/SQRT results instant (busy flag never set)                                                        | negligible                                                 |
| Display         | BG mode 6, display mode 3 (main-memory FIFO), ARM7 VBlank DMA, GBA-slot & wireless DMA `todo!()`/warn | affected features unimplemented                            |
| Timers          | reads race-free by construction (lazy counter) — hardware read-during-increment glitches not modeled  | none practical                                             |

---

## 12. File Index

| Block        | Files                                                                                                                                                                                                                                                                                        |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Top level    | [core/src/nds.rs](../../core/src/nds.rs), [core/src/hw.rs](../../core/src/hw.rs)                                                                                                                                                                                                             |
| Scheduler    | [core/src/hw/scheduler.rs](../../core/src/hw/scheduler.rs)                                                                                                                                                                                                                                   |
| GPU shared   | [core/src/hw/gpu.rs](../../core/src/hw/gpu.rs), [gpu/registers.rs](../../core/src/hw/gpu/registers.rs), [gpu/vram.rs](../../core/src/hw/gpu/vram.rs)                                                                                                                                         |
| 2D engines   | [gpu/engine2d.rs](../../core/src/hw/gpu/engine2d.rs), [gpu/engine2d/registers.rs](../../core/src/hw/gpu/engine2d/registers.rs)                                                                                                                                                               |
| 3D engine    | [gpu/engine3d.rs](../../core/src/hw/gpu/engine3d.rs), [geometry.rs](../../core/src/hw/gpu/engine3d/geometry.rs), [rendering.rs](../../core/src/hw/gpu/engine3d/rendering.rs), [math.rs](../../core/src/hw/gpu/engine3d/math.rs), [registers.rs](../../core/src/hw/gpu/engine3d/registers.rs) |
| Sound        | [core/src/hw/spu.rs](../../core/src/hw/spu.rs), [spu/registers.rs](../../core/src/hw/spu/registers.rs), [spu/audio.rs](../../core/src/hw/spu/audio.rs)                                                                                                                                       |
| DMA / Timers | [core/src/hw/dma.rs](../../core/src/hw/dma.rs), [core/src/hw/timers.rs](../../core/src/hw/timers.rs)                                                                                                                                                                                         |
| IPC / IRQ    | [core/src/hw/ipc.rs](../../core/src/hw/ipc.rs), [core/src/hw/interrupt_controller.rs](../../core/src/hw/interrupt_controller.rs)                                                                                                                                                             |
| Memory       | [core/src/hw/mem.rs](../../core/src/hw/mem.rs), [mem/arm7.rs](../../core/src/hw/mem/arm7.rs), [mem/arm9.rs](../../core/src/hw/mem/arm9.rs), [mem/cp15.rs](../../core/src/hw/mem/cp15.rs)                                                                                                     |
| CPU          | [core/src/arm.rs](../../core/src/arm.rs), [arm/arm.rs](../../core/src/arm/arm.rs), [arm/thumb.rs](../../core/src/arm/thumb.rs), [arm/registers.rs](../../core/src/arm/registers.rs)                                                                                                          |
| Cartridge    | [core/src/hw/cartridge.rs](../../core/src/hw/cartridge.rs), [header.rs](../../core/src/hw/cartridge/header.rs), [key1_encryption.rs](../../core/src/hw/cartridge/key1_encryption.rs), [backup.rs](../../core/src/hw/cartridge/backup.rs)                                                     |
| Peripherals  | [spi.rs](../../core/src/hw/spi.rs), [spi/tsc.rs](../../core/src/hw/spi/tsc.rs), [rtc.rs](../../core/src/hw/rtc.rs), [keypad.rs](../../core/src/hw/keypad.rs), [math.rs](../../core/src/hw/math.rs)                                                                                           |
