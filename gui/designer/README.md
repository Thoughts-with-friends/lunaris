# Lunaris Designer

One binary (`designer.exe`) to play a game, open the console up as a
Vivado-style block design, inspect every block live, and plug your own FPGA
blocks into the DS.

```sh
cargo xtask run --gui designer --release -- path/to/game.nds
# or: npm run design
```

## Tabs

| Tab | What it does |
| --- | --- |
| **Emulate** | The game, as in `gui/egui`. Keys: X/Z = A/B, S/A = X/Y, Q/W = L/R, Enter = Start, Backspace = Select, arrows = D-pad; the mouse is the stylus. |
| **Design** | The console, drawn from vector shapes with both screens live. Its buttons (D-pad, A/B/X/Y, L/R, START/SELECT) are pressed with the mouse and the bottom screen is touchable, so the game is playable here too. Drag the background to move it, wheel or pinch to zoom, two-finger scroll to move; **Reset view** restores it. Double-click the console (off the controls) and the view zooms inside to the block design, with the clicked part's block selected; **< Back to DS** (top left of the design) returns. |

Emulation keeps running in both tabs.

## Block design

```text
 ARM9 / DMA9 --AHB--> ARM9 Interconnect --> RAMs, VRAM, 2D/3D, timers, IPC, Slot-1, Slot-2 (FPGA), ...
 ARM7 / DMA7 --AHB--> ARM7 Interconnect --> RAMs, SPU, SPI, RTC, Wi-Fi, timers, IPC, ...
 sources --IRQ--> IRQ9 / IRQ7 --nIRQ--> CPUs        VRAM -> 2D A/B, 3D -> LCDs        SPU -> speakers
```

* Drag blocks to move them, drag the background or two-finger scroll to pan,
  wheel or pinch to zoom.
  **Save layout** writes `instances/instance1/designer_layout.json` (also saved on exit).
* Bus nets are thick with Vivado-style interface pins; interrupt nets turn
  orange while pending in IF and red while IF & IE are set.
* Moving dots on a net: the registers of one of its blocks changed recently.
* Hover a net for its endpoints and (for interrupts) live IF/IE/IME; click it
  to see the registers of both ends.

## Properties panel

Every block has a **Registers** tab: I/O address, name, value (changed values
glow), and the GBATEK bit fields (click a register to expand). Values are read
from core state directly, so inspecting never pops a FIFO or consumes a
cartridge word.

| Block | Extra view |
| --- | --- |
| Speakers, SPU | stereo waveform, spectrum, scrolling spectrogram (FFT of `Console::audio_tap`), 16 channels decoded |
| VRAM | banks A-I: VRAMCNT, decoded mapping, bank contents |
| Main RAM, WRAM, ARM7 WRAM | hex dump with "go to address" |
| ARM9, ARM7 | R0-R15, CPSR mode/flags, halted |
| LCDs, 2D engines, touch screen | the screen they drive (follows POWCNT1 bit 15) |
| Slot-2 / FPGA | see below |

## FPGA on Slot-2

The FPGA cartridge sits on the GBA slot bus (`0800_0000h..0A01_0000h`, owned
by the CPU EXMEMCNT bit 7 selects); its interrupt is ARM9 IF bit 13. It is
not inserted by default because games probe the GBA slot.

Two ways to describe a block; both run inside the emulator:

```text
 core/fpga/hdl/<project>/             one folder per project
   <name>.json  BlockSpec (register map + behaviour)
      |-- SpecCore              reference model
      `-- codegen -> .v .sv .rs  generated, same pins and semantics
   <name>.v / .sv  hand-written RTL
      `-- built-in Verilog simulator (VerilogCore), no external tools
 pins: core/fpga/src/signals.rs   clock-edge semantics: core/fpga/src/core.rs
```

Bundled projects:

| Folder | What |
| --- | --- |
| `hdl/fpga_timer/` | JSON spec + its generated Verilog / SystemVerilog / Rust: counter, compare IRQ |
| `hdl/toggle_switch/` | Verilog sample 1: push-on/push-off switch, LED, press counter, IRQ |
| `hdl/riscv_rv32i/` | Verilog sample 2: single-cycle RV32I CPU, 4 KiB IMEM + 4 KiB DMEM, mailbox to/from the DS, IRQ on halt / mailbox |

How a ROM uses a block: ordinary loads and stores to `0x0800_0000 + base +
offset` from the CPU that owns the slot (EXMEMCNT bit 7), and the Slot-2
interrupt (IF bit 13). For the RISC-V sample the DS writes a program into
IMEM (`+0x1000`), writes `CTRL = 3` (reset + run), and reads `MAILBOX_OUT`
(`+0x10`) or waits for the interrupt.

In the FPGA view: insert the cartridge, add a sample or load a `.json` /
`.v` / `.sv`, watch and write registers, press the toggle switch (LED shown),
load and run RISC-V programs (Fibonacci and counter demos, or `.bin` / `.hex`),
read the bus log, and view the RTL or the generated sources.

The built-in simulator handles the synthesizable subset listed in
`core/fpga/src/verilog/mod.rs` (one module, vectors up to 64 bits, memories,
`always @(posedge clk)` / `@(*)`, `always_ff` / `always_comb`, if / case, all
operators). A `// ds_fpga: size=0x4000 clock_div=1024` comment sets the window
and clock; the register list comes from `localparam ADDR_<NAME>` constants.
It runs about 300k clocks/s for the RISC-V sample in a release build, so
that sample defaults to 33.5 MHz / 1024.

Command line generation from a spec:

```sh
cargo run -p ds_fpga --example fpga_gen -- my_block.json core/fpga/hdl/my_block/
```

Tested: the generated Rust, and the generated Verilog and SystemVerilog run by
the built-in simulator, all match the reference model cycle for cycle under
random bus traffic; the RISC-V sample passes an RV32I instruction test and
Fibonacci, also driven by ARM9 loads/stores through the emulator bus.

### Waveform window

While an FPGA block exists, the right edge of the window shows its signals
scrolling as the ROM runs, like a simulator's wave view: every signal of a
Verilog block (every register of a spec block), one sample per FPGA clock,
with a drawn `clk`. Wheel zooms, drag pans (stops following), double-click
follows again, the hover line shows all values at that clock, clicking a
name hides it. Toggle it with Emulation > Waveform window; recording runs
only while it is shown. Each signal keeps its last 4096 changes.

### Where this is going

`FpgaCore` (`eval` with the clock low, `posedge`, `idle`) is the seam for
other implementations of a block: a Verilator backend for full-language,
faster RTL; more behaviour kinds in `BlockSpec`; DMA access to Slot-2 (CPU
accesses only today); the RISC-V core mastering the DS bus; and the same
pattern for replacing larger blocks (CPU, GPU).

## Unattended screenshots

`LUNARIS_DESIGNER_START=console|design|design:<block id>|fpga|riscv` opens on that
view; `LUNARIS_DESIGNER_NO_INPUT=1` ignores the host mouse/keyboard and opens
the window off the desktop, so it can be captured with `PrintWindow`.
