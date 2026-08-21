# Lunaris: A Nintendo DS Emulator, Chapter by Chapter

A complete functional specification of the **Lunaris** Nintendo DS emulator,
cross-referenced against [GBATEK](https://problemkaputt.de/gbatek.htm) and the
Lunaris source tree.

Chapter 1 answers _how do you build a DS emulator at all_; Chapters 2–20 go
through every subsystem in the order you would implement it, showing the
essential code and linking to where it lives.

Each chapter carries: the hardware it models, a verified GBATEK reference, the
Lunaris implementation with excerpts and file links, text diagrams of the data
structures and protocols, and an honest list of what is **not** implemented.

---

## Contents

| #   | Chapter                                                             | Covers                                                                     |
| --- | ------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 1   | [Building a Nintendo DS Emulator](01_emulator_architecture.md)      | What a DS is, timing models, the main loop, implementation order           |
| 2   | [Workspace and Code Layout](02_workspace_layout.md)                 | Crate graph, conventions, `bitfield!`, savestate derives                   |
| 3   | [The ARM CPU Cores](03_arm_cpu.md)                                  | ARM7TDMI + ARM946E-S, pipeline, dispatch tables, IRQ entry                 |
| 4   | [CP15, TCM and the Protection Unit](04_cp15_and_tcm.md)             | Coprocessor 15, ITCM/DTCM, `WAIT_FOR_IRQ`                                  |
| 5   | [Memory Map and Page Tables](05_memory_map.md)                      | The two memory maps, raw-pointer page tables, WRAMCNT                      |
| 6   | [The Scheduler and Timers](06_scheduler_and_timers.md)              | The event min-heap, 4+4 hardware timers, cascade mode                      |
| 7   | [Interrupts and IPC](07_interrupts_and_ipc.md)                      | IE/IF/IME, IPCSYNC, the IPC FIFOs, crossed IRQ routing                     |
| 8   | [DMA](08_dma.md)                                                    | 4+4 channels, start occasions, latching, the timing gap                    |
| 9   | [The 2D Graphics Engines](09_2d_engine.md)                          | ROM→VRAM→screen data path, tile format, BG modes, sprites, UI, compositing |
| 10  | [The 3-D Geometry Engine](10_3d_geometry.md)                        | GXFIFO, matrix stacks, vertex assembly, culling, clipping                  |
| 11  | [The 3-D Rasteriser](11_3d_rasterizer.md)                           | Scan conversion, perspective interpolation, textures, fog                  |
| 12  | [VRAM Banking and Display Output](12_vram_and_display.md)           | The nine banks, LCD timing, display capture, POWCNT1                       |
| 13  | [The Sound Processing Unit](13_spu.md)                              | 16 channels, ADPCM, mixing, sound capture, host clock pacing               |
| 14  | [The Cartridge and Boot](14_cartridge_and_boot.md)                  | ROM layout, the serial protocol, KEY1, direct boot                         |
| 15  | [Backup Memory and Save Files](15_backup_memory.md)                 | The game database, EEPROM/Flash, the IR carts, `.dsv`                      |
| 16  | [SPI: Firmware, Touchscreen, Power](16_spi_firmware_touchscreen.md) | The SPI bus, firmware patching, the TSC2046                                |
| 17  | [RTC, Keypad and the Maths Units](17_rtc_keypad_math.md)            | The bit-banged RTC, active-low buttons, div/sqrt edge cases                |
| 18  | [Wi-Fi and Local Multiplayer](18_wifi_and_local_mp.md)              | The W_ registers, MP sequence diagrams, the wireless timebase              |
| 19  | [Savestates](19_savestates.md)                                      | What cannot be serialised, and the bugs that proved it                     |
| 20  | [Cheats, Debug Tools and Frontends](20_cheats_debug_frontend.md)    | The AR interpreter, diagnostics, debug viewers, the GUI boundary           |

---

## Reading paths

```text
   "I want to write my own DS emulator"
        1 → 3 → 4 → 5 → 6 → 7 → 14 → 8 → 12 → 9 → 16 → 17 → 15 → 13 → 10 → 11

   "I want to understand Lunaris specifically"
        1 → 2 → then whichever subsystem you are touching

   "Something is broken and I need to find it"
        black screen ......... 9, 12, and 20 §20.2 (diagnostics)
        freeze ............... 10 §10.3, 19 §19.4
        no audio ............. 13 §13.7
        saves lost ........... 15
        multiplayer .......... 18
```

---

## Conventions

- **Source links** are workspace-root-relative, e.g.
  [nds.rs:216](core/src/nds.rs#L216).
- **GBATEK links** are only used where the anchor is cited in the Lunaris source
  itself, so they are verified rather than guessed.
- **Divergences** — anything Lunaris does not implement, or implements
  differently from hardware — get their own section at the end of each chapter,
  with a pointer to the melonDS file that does implement it where one exists.
- Code excerpts are verbatim except where condensed to fit; elisions are marked
  `// ...` and condensed passages `/* ... */`.

melonDS is used throughout as the reference implementation. A copy is vendored
at [docs/design/melonds/](docs/design/melonds/) for comparison.
