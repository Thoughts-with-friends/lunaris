# 1. Building a Nintendo DS Emulator — the Lunaris Architecture

This chapter answers the first question anyone writing a console emulator has to
answer: **what, exactly, are we simulating, and in what order?** Everything in
the following chapters is a detail hanging off the skeleton described here.

GBATEK reference: [DS Technical Data](https://problemkaputt.de/gbatek.htm#dstechnicaldata)

---

## 1.1 What a Nintendo DS actually is

A DS is not one machine. It is **two CPUs sharing one bus**, plus a pile of
memory-mapped peripherals, driven by a single master clock.

```text
                          ┌───────────────────────────────────────────┐
                          │        Master clock 33.513982 MHz         │
                          └───────────────┬───────────────────────────┘
                                          │
        ┌─────────────────────────────────┴──────────────────────────────┐
        │                                                                │
┌───────▼────────┐                                              ┌────────▼───────┐
│  ARM946E-S     │  "ARM9"                                      │   ARM7TDMI     │
│  67 MHz  (2x)  │                                              │  33 MHz  (1x)  │
│                │                                              │                │
│  game logic    │                                              │  audio, Wi-Fi, │
│  3D + 2D setup │                                              │  touchscreen,  │
│  main RAM      │                                              │  RTC, firmware │
│  CP15 + TCM    │                                              │  (a GBA CPU)   │
└───────┬────────┘                                              └────────┬───────┘
        │                                                                │
        │         ┌───────────────── shared bus ──────────────────┐      │
        └─────────┤                                               ├──────┘
                  │  Main RAM 4 MB   Shared WRAM 32 KB   IPC FIFO │
                  │  VRAM 656 KB     Cartridge bus       I/O regs │
                  └───────────────────────────────────────────────┘
                          │                       │
              ┌───────────▼─────────┐   ┌─────────▼──────────┐
              │ 2D engines A and B  │   │  SPU: 16 channels  │
              │ 3D geometry + raster│   │  + 2 capture units │
              └───────────┬─────────┘   └─────────┬──────────┘
                          │                       │
                   ┌──────▼──────┐          ┌─────▼─────┐
                   │ 2 x 256x192 │          │  Speakers │
                   │    LCDs     │          └───────────┘
                   └─────────────┘
```

The ARM9 is the "application" processor; the ARM7 is a service processor
inherited almost verbatim from the Game Boy Advance. They do not share
registers or caches — they communicate through the **IPC FIFO** and through
shared memory (Chapter 7).

Lunaris models this literally. [nds.rs:18-24](core/src/nds.rs#L18-L24):

```rust
pub struct NDS {
    /// ARM7TDMI sub-processor (audio, Wi-Fi, I/O assist).
    arm7: ARM<false>,
    /// ARM946E-S main processor (game code, 3D, main memory).
    arm9: ARM<true>,
    hw: HW,
}
```

Two CPU cores, one `HW` that owns _everything else_. The const generic
`IS_ARM9` selects the core's behaviour at compile time, so the two cores share
one body of code with no runtime branching on "which CPU am I?".

---

## 1.2 The three strategies for emulating a console

Before writing any code you pick a timing model. There are three, and they
trade accuracy against speed:

```text
 accuracy ▲
          │                                          ┌──────────────────────┐
          │                                          │ 3. Cycle-stepped     │
          │                                          │    Every component   │
          │                                          │    ticks once per    │
          │                                          │    clock cycle.      │
          │                                          │    Slow, simple,     │
          │                                          │    very accurate.    │
          │                    ┌─────────────────────┴──────────────────────┘
          │                    │ 2. Event-scheduled  │
          │                    │    Run the CPU to   │  ◀── Lunaris is here
          │                    │    the next event,  │
          │                    │    then service it. │
          │ ┌──────────────────┴─────────────────────┘
          │ │ 1. Frame-stepped │
          │ │    Run N cycles, │
          │ │    then poll all │
          │ │    peripherals.  │
          │ └──────────────────┘
          └──────────────────────────────────────────────────────► speed
```

Lunaris uses **(2) event scheduling**, the same model melonDS and most modern
emulators use. A min-heap holds "at cycle _X_, do _Y_"; the CPUs are allowed to
run freely up to the cycle of the nearest pending event, and only then does the
emulator stop to service hardware. See Chapter 6.

---

## 1.3 The main loop

The whole emulator, in eighteen lines. [nds.rs:216-236](core/src/nds.rs#L216-L236):

```rust
pub fn emulate_frame(&mut self) {
    while !self.hw.rendered_frame() {
        if likely(!self.hw.gpu.bus_stalled()) {
            let cycle = self.hw.cycle();
            // The max cycle desync was ~30 when the CPUs were running tightly
            let target = std::cmp::min(cycle + 30, self.hw.cycle_at_next_event());

            self.arm9.emulate(&mut self.hw, target * 2);
            self.arm7.emulate(&mut self.hw, target);
            self.hw.clock_until(target);
        } else {
            self.hw.clock_until_event();
            self.arm9.set_cycle(self.hw.cycle() * 2);
            self.arm7.set_cycle(self.hw.cycle());
        }
    }

    if self.hw.enable_cheats {
        self.hw.apply_cheats();
    }
}
```

Unrolled as a timeline, one iteration looks like this:

```text
 master cycle ──────────────────────────────────────────────────────────────►
              C                                     C+30 or next event
              │                                          │
   ARM9 ──────█████████████████████████████████████████████   (runs to 2*target)
              │                                          │
   ARM7 ──────████████████████████████████████████████████    (runs to target)
              │                                          │
   HW   ──────┴──────────────────────────────────────────█ clock_until(target)
                                                         │
                                                         └─► fire due events:
                                                             HBlank / DMA /
                                                             TimerOverflow /
                                                             audio sample ...
```

Three design decisions are visible in that snippet, and each is a lesson:

**(a) The 30-cycle slice cap.** `min(cycle + 30, next_event)` means the two
cores never diverge by more than ~30 master cycles even when the next scheduled
event is far away. Without the cap, ARM9 could run milliseconds ahead of ARM7
and observe IPC / shared-RAM writes in the wrong order. The cap is the cheapest
possible substitute for true interleaved execution.

**(b) ARM9 gets `target * 2`.** The ARM9 counts cycles at twice the master
clock rate, so its budget is doubled rather than kept in a separate unit. Every
cycle number in the emulator is _master cycles_, except inside `ARM<true>`.

**(c) The stall path.** When the 3-D geometry engine backpressures the bus
(`bus_stalled()`), running the CPUs would be wrong — the real hardware freezes
the ARM9 on a full geometry FIFO. Lunaris instead advances the clock straight to
the next event and _re-synchronises both CPU counters to the new time_. Skipping
that resync is a classic emulator bug: the CPUs would then believe they still
owe millions of cycles of work.

---

## 1.4 Frame — what the frontend sees

`emulate_frame` returns when the GPU has produced a complete frame. The
frontend then pulls two 256×192 framebuffers of BGR555 pixels:

```text
   frontend (gui/egui)                    core (nds-core)
   ──────────────────                     ───────────────
   loop {
     poll input      ─── press_key ──────►  Keypad regs (Ch. 17)
     nds.emulate_frame() ───────────────►  run until VBlank
     nds.get_screens()  ◀───────────────   [&Vec<u16>; 2]   (Ch. 12)
     upload to GPU texture
     audio ring buffer  ◀───────────────   SPU mixes       (Ch. 13)
     present at 59.8262 Hz
   }
```

[`get_screens`](core/src/nds.rs#L256-L260) is deliberately a _borrow_, not a
copy — the frontend uploads straight from core memory. Nintendo DS refresh is
59.8262 Hz, not 60; pacing to exactly 60 Hz produces audible pitch drift, which
is why Lunaris can pace on the audio clock instead
([`set_audio_sync`](core/src/nds.rs#L84-L87), Chapter 13).

---

## 1.5 The order to implement things in

If you are writing your own emulator, this is a working order — each step is
testable before the next one exists:

```text
 ┌────────────────────────────────────────────────────────────────────┐
 │ 1. ARM7TDMI + ARM946E-S interpreter          → Chapters 3, 4       │
 │    Validate against a CPU test ROM before anything else.           │
 ├────────────────────────────────────────────────────────────────────┤
 │ 2. Memory map + page tables                  → Chapter 5           │
 │    Wrong mirroring here looks like "random" CPU bugs later.        │
 ├────────────────────────────────────────────────────────────────────┤
 │ 3. Scheduler, timers, interrupts, IPC        → Chapters 6, 7       │
 │    Now a ROM can boot and spin in its main loop.                   │
 ├────────────────────────────────────────────────────────────────────┤
 │ 4. Cartridge + direct boot                   → Chapter 14          │
 │    Skip the firmware menu; jump straight to the game's entry point.│
 ├────────────────────────────────────────────────────────────────────┤
 │ 5. DMA                                       → Chapter 8           │
 │    Nothing renders until DMA works; games move everything with it. │
 ├────────────────────────────────────────────────────────────────────┤
 │ 6. VRAM banking + 2D engines                 → Chapters 9, 12      │
 │    First picture on screen. Enormous morale boost.                 │
 ├────────────────────────────────────────────────────────────────────┤
 │ 7. Input, SPI/touchscreen, RTC               → Chapters 16, 17     │
 │    Now it is playable.                                             │
 ├────────────────────────────────────────────────────────────────────┤
 │ 8. Backup memory (save files)                → Chapter 15          │
 │    Playable *and* the player keeps their progress.                 │
 ├────────────────────────────────────────────────────────────────────┤
 │ 9. SPU                                       → Chapter 13          │
 ├────────────────────────────────────────────────────────────────────┤
 │10. 3-D geometry + rasteriser                 → Chapters 10, 11     │
 │    The single largest subsystem. Leave it for last.                │
 ├────────────────────────────────────────────────────────────────────┤
 │11. Savestates, Wi-Fi, debug tools            → Chapters 18, 19, 20 │
 └────────────────────────────────────────────────────────────────────┘
```

Lunaris was built roughly in this order, and the chapter numbering follows it.

---

## 1.6 Boot: direct boot vs. firmware boot

A real DS powers on into the firmware menu, which then loads the cartridge.
Emulators usually offer a shortcut called **direct boot**: copy the ARM7 and
ARM9 binaries out of the cartridge into RAM yourself, set the registers to the
values the firmware would have left, and jump to the game's entry point.

```text
  real hardware                          Lunaris direct boot
  ─────────────                          ───────────────────
  reset vector                           HW::new(..., direct_boot = true)
      │                                       │
  ARM7 BIOS                              copy header.arm9_size bytes
      │                                  from rom[arm9_rom_offset]
  firmware (SPI flash)                   to header.arm9_ram_addr
      │                                       │  (same for ARM7)
  user menu / health screen              write firmware setting stubs
      │                                       │  (e.g. 23FFC80h = 5)
  KEY1 auth + secure area load           set PC = header.arm9_entry_addr
      │                                       │
      ▼                                       ▼
  game entry point                       game entry point
```

Lunaris hardcodes `direct_boot = true` in
[`NDS::new`](core/src/nds.rs#L61-L71), and the actual copy is
[`HW::init_arm9`](core/src/hw/mem/arm9.rs#L183-L196). The cartridge KEY1
encryption machinery still exists (Chapter 14) because games re-read their own
secure area at runtime.

---

## 1.7 Where the rest of this series goes

| Chapters | Subject                                                      |
| -------- | ------------------------------------------------------------ |
| 2        | Crate layout, how the code is organised, conventions         |
| 3–5      | CPU cores, CP15/TCM, memory map and page tables              |
| 6–8      | Scheduler, timers, interrupts, IPC, DMA                      |
| 9–12     | Graphics: 2D engines, 3-D geometry, rasteriser, VRAM/display |
| 13       | Sound                                                        |
| 14–17    | Cartridge, saves, SPI/firmware/touchscreen, RTC/keypad/maths |
| 18       | Wi-Fi and local multiplayer                                  |
| 19–20    | Savestates, cheats, debug tooling, frontends                 |

---

[Next: 2. Workspace and Code Layout →](02_workspace_layout.md)
