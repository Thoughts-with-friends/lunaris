# 2. Workspace and Code Layout

Before diving into hardware, it is worth knowing where everything lives. This
chapter is the map you will keep coming back to while reading Chapters 3–20.

---

## 2.1 The crate graph

Lunaris is a Cargo workspace of eight crates. The rule that shapes the whole
tree is: **`nds-core` knows nothing about windows, files pickers, sockets, or
audio devices.** It is a pure state machine with a `emulate_frame()` button.

```text
   ┌─────────────────────────────────────────────────────────────────────┐
   │                          F R O N T   E N D S                        │
   │                                                                     │
   │  gui/egui        gui/imgui        gui/melon_egui                    │
   │  (default)       (legacy)         (melonDS-core reference build)    │
   │      │                │                    │                        │
   │      └────────┬───────┘                    │                        │
   │               ▼                            ▼                        │
   │         gui/common                    melonds-rs (external)         │
   │   config, savestate container,                                      │
   │   framebuffer math, upscaling,                                      │
   │   input mapping, ROM loader                                         │
   │               │                                                     │
   │               │        gui/net ── LAN rooms, netplay transport      │
   └───────────────┼─────────────┬───────────────────────────────────────┘
                   ▼             ▼
            ┌──────────────────────────────────┐
            │            nds-core              │   the emulator proper
            │  arm/  ── two CPU cores          │
            │  hw/   ── every peripheral       │
            └───────┬──────────────┬───────────┘
                    │              │
          ┌─────────▼──────┐  ┌────▼──────────────┐
          │   bitfield     │  │    free_bios      │
          │ proc-macro for │  │ open-source BIOS  │
          │ register decls │  │ + firmware blobs  │
          └────────────────┘  └───────────────────┘

          xtask ── `cargo xtask run --gui egui|imgui|melon`
```

| Crate                | Path                                           | Role                                                                                         |
| -------------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `nds-core`           | [core/](core/)                                 | The emulator. Everything in Chapters 3–19.                                                   |
| `bitfield`           | [bitfield/src/lib.rs](bitfield/src/lib.rs)     | Proc macro turning `struct DISPCNT: u32 { bg_mode: u8 @ 0..=2, … }` into accessors.          |
| `free_bios`          | [free_bios/src/lib.rs](free_bios/src/lib.rs)   | Bundled ARM7/ARM9 BIOS and firmware images so the emulator runs with no user-supplied dumps. |
| `lunaris_gui_common` | [gui/common/src/lib.rs](gui/common/src/lib.rs) | Backend-neutral frontend helpers.                                                            |
| `lunaris-egui`       | [gui/egui/](gui/egui/)                         | The default frontend (`default-members`).                                                    |
| `lunaris` (imgui)    | [gui/imgui/](gui/imgui/)                       | The original Dear ImGui frontend.                                                            |
| `melon_egui`         | [gui/melon_egui/](gui/melon_egui/)             | Same egui shell driving the real melonDS core, used as a behavioural reference.              |
| `lunaris_net`        | [gui/net/](gui/net/)                           | LAN room discovery and the netplay transport.                                                |
| `xtask`              | [xtask/src/main.rs](xtask/src/main.rs)         | `cargo xtask` build/run dispatcher for the three GUIs.                                       |

---

## 2.2 Inside `nds-core`

```text
core/src/
├── lib.rs               entry point, re-exports NDS, foreign-save normalisation
├── nds.rs               NDS struct, emulate_frame, the whole public API
├── macros.rs            impl_savestate_bitflags!
├── arm.rs               shared CPU core (generic over IS_ARM9)     ── Ch. 3
│   └── arm/
│       ├── arm.rs       ARM (32-bit) instruction implementations
│       ├── thumb.rs     THUMB (16-bit) instruction implementations
│       ├── instructions.rs   dispatch-table plumbing
│       └── registers.rs      CPSR/SPSR, banked register file
└── hw.rs                the HW aggregate: owns all peripherals
    └── hw/
        ├── mem.rs       read/write dispatch          ── Ch. 5
        │   └── mem/{arm7,arm9}.rs   per-CPU page tables & I/O maps
        │       cp15.rs             CP15, TCM, protection unit  ── Ch. 4
        ├── scheduler.rs event min-heap                ── Ch. 6
        ├── timers.rs    4+4 hardware timers           ── Ch. 6
        ├── interrupt_controller.rs  IE/IF/IME         ── Ch. 7
        ├── ipc.rs       IPC sync + FIFO               ── Ch. 7
        ├── dma.rs       4+4 DMA channels              ── Ch. 8
        ├── gpu.rs       LCD timing, display control   ── Ch. 12
        │   └── gpu/
        │       ├── engine2d.rs, engine2d/registers.rs  ── Ch. 9
        │       ├── engine3d.rs
        │       │   └── engine3d/{geometry,rendering,registers,math}.rs
        │       │                                       ── Ch. 10, 11
        │       ├── vram.rs   9 banks, 9 mapping targets ── Ch. 12
        │       └── debug.rs  palette/tile/map viewers   ── Ch. 20
        ├── spu.rs       16 channels + capture          ── Ch. 13
        ├── cartridge.rs ROM protocol, KEY1             ── Ch. 14
        │   └── cartridge/{header,key1_encryption,backup}.rs
        │       backup/{eeprom,flash,ir,no_backup,game_db}.rs  ── Ch. 15
        ├── spi.rs       firmware + touchscreen + power ── Ch. 16
        ├── rtc.rs, keypad.rs, math.rs                  ── Ch. 17
        ├── net.rs       Wi-Fi, local MP                ── Ch. 18
        │   └── net/{wifi/,local/,mp_interface.rs,bridge.rs}
        └── ar.rs        Action Replay cheat VM         ── Ch. 20
```

---

## 2.3 Two conventions you will see everywhere

### `bitfield!` for hardware registers

DS I/O registers are dense bit-packed words. Writing `(value >> 5) & 0x7`
by hand across 300 registers is how emulators get subtle bugs. The `bitfield`
proc macro declares them structurally instead
([arm/registers.rs:17-32](core/src/arm/registers.rs#L17-L32)):

```rust
bitfield! {
    #[derive(emu_utils::Savestate)]
    #[derive(Debug, PartialEq, Clone, Copy)]
    struct StatusRegBits: u32 {
        n: bool @ 31,
        z: bool @ 30,
        c: bool @ 29,
        v: bool @ 28,
        q: bool @ 27,
        _: _ @ 8..=26,
        i: bool @ 7,
        f: bool @ 6,
        t: bool @ 5,
        mode: u8 @ 0..=4,
    }
}
```

That mirrors the GBATEK register table line for line, which makes review
against the spec mechanical:

```text
  CPSR — Current Program Status Register
  ┌───┬───┬───┬───┬───┬───────────────────────┬───┬───┬───┬───────────┐
  │31 │30 │29 │28 │27 │        26 .. 8        │ 7 │ 6 │ 5 │  4 .. 0   │
  ├───┼───┼───┼───┼───┼───────────────────────┼───┼───┼───┼───────────┤
  │ N │ Z │ C │ V │ Q │       reserved        │ I │ F │ T │   Mode    │
  └───┴───┴───┴───┴───┴───────────────────────┴───┴───┴───┴───────────┘
    │   │   │   │   │                           │   │   │       │
    │   │   │   │   └ sticky overflow           │   │   │       └ USR/FIQ/IRQ/
    │   │   │   └ overflow           IRQ disable┘   │   │          SVC/ABT/SYS/UND
    │   │   └ carry                       FIQ disable   └ THUMB state
    │   └ zero
    └ negative
```

### `#[derive(emu_utils::Savestate)]` on every stateful struct

Savestates are not bolted on at the end in Lunaris; every hardware struct
derives serialisation as it is written. Fields that must _not_ be serialised
(BIOS images, raw pointers, host handles) are opted out explicitly
([hw.rs:70-100](core/src/hw.rs#L70-L100)):

```rust
#[derive(emu_utils::Savestate)]
#[load(post = "self.post_load_hw(save)?", in_place_only)]
pub struct HW {
    #[savestate(skip)]
    pub enable_cheats: bool,
    // ...
    /// Not serialized: BIOS images are immutable and re-supplied by the host
    /// at construction time, so shipping ~20 KB of BIOS in every savestate
    /// is pure waste.
    #[savestate(skip)]
    bios7: Vec<u8>,
    // ...
    /// Raw-pointer page table for ARM9 memory (4 KiB pages). Not serialized.
    #[savestate(skip)]
    arm9_page_table: Vec<*mut u8>,
```

The `#[load(post = …)]` hook is what rebuilds those skipped pointer tables after
a load. Chapter 19 covers the whole mechanism, including the two bugs that
model produced (`usize` serialised as `u32`, and `Vec` in-place loads ignoring
the length prefix).

For types from `bitflags!` — which have a private `bits` field the derive
cannot see — [`impl_savestate_bitflags!`](core/src/macros.rs#L1-L41) provides
the three impls by hand.

---

## 2.4 Cycle counters are `usize`, serialised as `u64`

A recurring theme. Every cycle counter in the emulator is an **absolute**
master-clock count that only ever grows:

```text
   ARM7 counter @ 33,513,982 Hz  ─►  u32 overflow after ~128 s of play
   ARM9 counter @ 67,027,964 Hz  ─►  u32 overflow after  ~64 s of play
```

`emu-utils` stores a `usize` as `u32`. Left alone, that silently truncates a
savestate taken after a minute of play, and the emulator freezes on load
because every scheduled event is suddenly in the distant past. Lunaris
overrides the storage type at each counter ([arm.rs:42-54](core/src/arm.rs#L42-L54)):

```rust
#[derive(emu_utils::Savestate)]
pub struct ARM<const IS_ARM9: bool> {
    /// Serialized as `u64`: emu-utils stores `usize` as `u32`, which silently
    /// truncates this absolute cycle counter after ~64s (ARM9) / ~128s (ARM7)
    /// of real play.
    #[store(with = "save.store(&mut (*cycle as u64))?")]
    #[load(
        with = "save.load::<u64>()? as usize",
        with_in_place = "*cycle = save.load::<u64>()? as usize"
    )]
    cycle: usize,
```

The same treatment is applied to the scheduler
([scheduler.rs:73-96](core/src/hw/scheduler.rs#L73-L96)) and to timer start
cycles.

---

## 2.5 Building and running

```text
  cargo run                        →  gui/egui (default-members)
  cargo xtask run  --gui egui      →  same, explicit
  cargo xtask run  --gui imgui     →  the Dear ImGui frontend
  cargo xtask build --gui melon    →  melonDS-core reference build
                                       (locates an LLVM toolchain for the
                                        C++ side automatically)
```

`xtask` exists because Cargo has no syntax for a custom `--gui` flag; it simply
re-invokes `cargo <build|run> -p <package>`. See
[xtask/src/main.rs:1-17](xtask/src/main.rs#L1-L17).

---

[← 1. Building a Nintendo DS Emulator](01_emulator_architecture.md) | [Next: 3. The ARM CPU Cores →](03_arm_cpu.md)
