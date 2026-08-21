# 8. DMA

Games move almost nothing by hand. Sprites, tilemaps, audio buffers, decoded
video, geometry commands — all of it travels by DMA. Get DMA wrong and the
screen stays black no matter how correct the 2-D engine is.

GBATEK references:
[DS DMA transfers](https://problemkaputt.de/gbatek.htm#dsdmatransfers) ·
[GBA DMA transfers](https://problemkaputt.de/gbatek.htm#gbadmatransfers)

---

## 8.1 Four channels per CPU

```text
   ARM9 DMA                              ARM7 DMA
   ┌──────────────────────────┐          ┌──────────────────────────┐
   │ ch0  SAD DAD CNT  prio ▲ │          │ ch0  SAD DAD CNT  prio ▲ │
   │ ch1  SAD DAD CNT       │ │          │ ch1  SAD DAD CNT       │ │
   │ ch2  SAD DAD CNT       │ │          │ ch2  SAD DAD CNT       │ │
   │ ch3  SAD DAD CNT  prio ▼ │          │ ch3  SAD DAD CNT  prio ▼ │
   └──────────────────────────┘          └──────────────────────────┘
     40000B0h .. 40000DFh                  40000B0h .. 40000DFh

   Per channel:
     SAD  source address       (28 bits, or 27 on some ARM7 channels)
     DAD  destination address  (28 bits, or 27 on some ARM7 channels)
     CNT  count + control word
```

Address masks differ per channel, which Lunaris encodes at construction
([dma.rs:296-308](core/src/hw/dma.rs#L296-L308)):

```rust
            sad: Address::new(if is_nds9 {
                0x0FFF_FFFF
            } else {
                if num == 0 { 0x07FF_FFFF } else { 0x0FFF_FFFF }
            }),
            dad: Address::new(if is_nds9 {
                0x0FFF_FFFF
            } else {
                if num == 3 { 0x07FF_FFFF } else { 0x0FFF_FFFF }
            }),
```

ARM7 channel 0 cannot source from the GBA slot; channel 3 cannot write to it.

---

## 8.2 The control word

```text
   DMAxCNT (32-bit: count in the low half, control in the high half)

    31 30 29 28 27 26 25 24 23 22 21 20 19        16 15                 0
   ┌──┬──┬──┬─────────┬──┬──┬─────┬─────┬────────────┬───────────────────┐
   │EN│IR│  start     │  │32│ SAD │ DAD │  reserved  │   word count      │
   │  │  │  timing    │RP│  │ ctrl│ ctrl│            │                   │
   └──┴──┴────────────┴──┴──┴─────┴─────┴────────────┴───────────────────┘
     │  │      │        │  │   │     │
     │  │      │        │  │   │     └── 0=increment 1=decrement 2=fixed
     │  │      │        │  │   │         3=increment + reload  (dest only)
     │  │      │        │  │   └──────── 0=increment 1=decrement 2=fixed
     │  │      │        │  └──────────── 0=16-bit  1=32-bit transfer
     │  │      │        └─────────────── repeat
     │  │      └──────────────────────── start occasion (see §8.3)
     │  └─────────────────────────────── IRQ on completion
     └────────────────────────────────── enable
```

---

## 8.3 Start occasions

A DMA does not run when it is enabled; it runs when its **trigger** occurs
([dma.rs:376-391](core/src/hw/dma.rs#L376-L391)):

```rust
pub enum Occasion {
    Immediate = 0,
    VBlank = 1,
    HBlank = 2,
    /// ARM9 only: triggered at the start of the display period. **Not yet implemented.**
    StartOfDisplay = 3,
    /// ARM9 only: triggered once per scanline for display capture. **Not yet implemented.**
    MainMemoryDisplay = 4,
    DSCartridge = 5,
    /// GBA cartridge slot DMA. **Not yet implemented.**
    GBACartridge = 6,
    /// ARM9 only: triggered when GXFIFO drops below half-full (< 128 entries).
    GeometryCommandFIFO = 7,
    /// ARM7 only: triggered by wireless interrupt. **Not yet implemented.**
    WirelessInterrupt = 8,
}
```

The ARM7 and ARM9 encode these differently in the register — the ARM7 has only
four options in a 2-bit field. `Occasion::val` and `Occasion::get` translate
both ways ([dma.rs:393-411](core/src/hw/dma.rs#L393-L411)), keeping the rest of
the emulator on a single unified enum.

```text
   trigger source                        who calls it
   ─────────────────────────────────     ────────────────────────────────
   Immediate      ─► run right now       Controller::write, at enable time
   VBlank         ─► once per frame      GPU VBlank handler
   HBlank         ─► once per scanline   GPU HBlank handler
   DSCartridge    ─► ROM word ready      cartridge transfer handler
   GeometryCmdFIFO─► GXFIFO half-empty   3-D engine, via check_geometry_…
```

### The `by_type` index

Scanning all four channels on every HBlank would be wasteful. Lunaris keeps a
reverse index from occasion to armed channel list
([dma.rs:18-27](core/src/hw/dma.rs#L18-L27)):

```rust
/// `by_type[occasion]` lists the channel indices currently armed for that
/// start trigger, enabling O(n_active) dispatch instead of scanning all four
/// channels on every potential trigger.
pub struct Controller {
    channels: [Channel; 4],
    pub by_type: [Vec<usize>; Occasion::num()],
}
```

```text
   by_type
   ┌──────────────────────┬───────────────┐
   │ Immediate       [0]  │ []            │
   │ VBlank          [1]  │ [1]           │  ch1 armed for VBlank
   │ HBlank          [2]  │ [2, 3]        │  ch2, ch3 armed for HBlank
   │ StartOfDisplay  [3]  │ []            │
   │ MainMemDisplay  [4]  │ []            │
   │ DSCartridge     [5]  │ [0]           │
   │ GBACartridge    [6]  │ []            │
   │ GeometryCmdFIFO [7]  │ []            │
   │ WirelessIRQ     [8]  │ []            │
   └──────────────────────┴───────────────┘

   HBlank fires  ─►  run_dmas_both(Occasion::HBlank)
                     ─► only channels 2 and 3 are even looked at
```

The index is maintained on every control write
([dma.rs:46-87](core/src/hw/dma.rs#L46-L87)):

```rust
pub fn write(&mut self, channel: usize, scheduler: &mut Scheduler, addr: u32, value: u8) {
    let prev_start_timing = self.channels[channel].cnt.start_timing;
    let prev_enable = self.channels[channel].cnt.enable;
    self.channels[channel].write(scheduler, (addr & 0xFF) as usize, value);
    let new_start_timing = self.channels[channel].cnt.start_timing;
    let new_enable = self.channels[channel].cnt.enable;
    // TODO: Only call this when the upper byte of cnt is written to
    if prev_enable != new_enable || prev_start_timing != new_start_timing {
        if prev_enable {
            let vec = &mut self.by_type[prev_start_timing as usize];
            let pos = vec.iter().position(|i| *i == channel);
            vec.swap_remove(pos.unwrap());
        }
        if new_enable {
            self.by_type[new_start_timing as usize].push(channel);
        }
    }
    if !prev_enable && new_enable {
        let channel = &mut self.channels[channel];
        channel.latch();
        // ... info! log ...
        match channel.cnt.start_timing {
            Occasion::Immediate => {
                scheduler.run_now(Event::DMA(channel.is_nds9, channel.num), HW::on_dma)
            }
            Occasion::GeometryCommandFIFO => scheduler.run_now(
                Event::CheckGeometryCommandFIFO,
                HW::check_geometry_command_fifo_handler,
            ),
            _ => (),
        }
    }
}
```

Two things to notice:

1. An **immediate** DMA is `run_now`, i.e. scheduled at the current cycle, not
   executed inline. That keeps the ordering of DMA against other events in one
   place — the scheduler.
2. A geometry-FIFO DMA schedules a _check_, not the transfer. The 3-D engine
   decides whether the FIFO actually has room (Chapter 10).

---

## 8.4 Latching

Real hardware copies SAD/DAD/count into internal registers when the channel is
enabled; later writes to the registers do not disturb a running transfer
([dma.rs:309-314](core/src/hw/dma.rs#L309-L314)):

```rust
pub fn latch(&mut self) {
    self.sad_latch = self.sad.addr & self.sad.mask;
    self.dad_latch = self.dad.addr & self.sad.mask;
    let count = self.cnt.count & self.cnt.count_mask;
    self.count_latch = if count == 0 { self.cnt.count_mask + 1 } else { count };
}
```

The `count == 0` case is the classic DMA quirk: a count of zero means
**maximum**, not "transfer nothing".

```text
   ARM9 channels:  count field 21 bits → 0 means 0x200000 words
   ARM7 ch0-2:     count field 14 bits → 0 means 0x4000 words
   ARM7 ch3:       count field 16 bits → 0 means 0x10000 words
```

---

## 8.5 Running a transfer

Dispatch picks the right memory accessors and width, monomorphised on the CPU
([dma.rs:118-156](core/src/hw/dma.rs#L118-L156)):

```rust
pub fn on_dma(&mut self, event: Event) {
    let (is_nds9, num) = match event {
        Event::DMA(is_nds9, num) => (is_nds9, num),
        _ => unreachable!(),
    };
    if self.dmas[is_nds9 as usize][num].cnt.transfer_32 {
        if is_nds9 {
            self.run_dma::<_, _, _, _, true>(
                num,
                &HW::arm9_get_access_time::<u32>,
                &HW::arm9_read::<u32>,
                &HW::arm9_write::<u32>,
            );
        } else {
            // ... arm7 u32 ...
```

Passing the read/write functions as generic parameters means the inner loop is
specialised — no per-word branch on "which CPU, what width".

The loop itself ([dma.rs:190-216](core/src/hw/dma.rs#L190-L216)):

```rust
let (addr_change, addr_mask) = if transfer_32 { (4, 0x3) } else { (2, 0x1) };
src_addr &= !addr_mask;
dest_addr &= !addr_mask;
let mut first = true;
let original_dest_addr = dest_addr;
let mut _cycles_passed = 0;
for _ in 0..count {
    let cycle_type = if first { AccessType::N } else { AccessType::S };
    _cycles_passed += access_time_fn(self, cycle_type, src_addr);
    _cycles_passed += access_time_fn(self, cycle_type, dest_addr);
    let value = read_fn(self, src_addr);
    write_fn(self, dest_addr, value);

    src_addr = match src_addr_ctrl {
        0 => src_addr.wrapping_add(addr_change),
        1 => src_addr.wrapping_sub(addr_change),
        2 => src_addr,
        _ => panic!("Invalid DMA Source Address Control!"),
    };
    dest_addr = match dest_addr_ctrl {
        0 | 3 => dest_addr.wrapping_add(addr_change),
        1 => dest_addr.wrapping_sub(addr_change),
        2 => dest_addr,
        _ => unreachable!(),
    };
    first = false;
}
```

```text
   Address control modes

   0 increment          src: A  A+2  A+4  A+6 …     dst: B  B+2  B+4 …
   1 decrement          src: A  A-2  A-4  A-6 …
   2 fixed              src: A  A    A    A   …     ← audio FIFO feeds
   3 increment + reload dst: B  B+2  B+4 … then back to B on repeat
                             ↑ only valid for the destination
```

Mode 3 ("increment/reload") is what a repeating HBlank DMA into a fixed
scanline buffer uses; the reload is applied after the loop
([dma.rs:217-223](core/src/hw/dma.rs#L217-L223)):

```rust
let channel = &mut self.dmas[i][num];
channel.sad_latch = src_addr;
channel.dad_latch = dest_addr;
if dest_addr_ctrl == 3 {
    channel.dad_latch = original_dest_addr
}
```

### Repeat, and disarming

Set before the loop runs ([dma.rs:178](core/src/hw/dma.rs#L178)):

```rust
channel.cnt.enable = channel.cnt.start_timing != Occasion::Immediate && channel.cnt.repeat;
```

An immediate DMA is always one-shot. A triggered DMA stays enabled only if
`repeat` is set; otherwise the channel is removed from `by_type`
([dma.rs:225-227](core/src/hw/dma.rs#L225-L227)):

```rust
if !channel.cnt.enable {
    self.dmas[i].disable(num)
}
```

### Completion interrupt

[dma.rs:233-244](core/src/hw/dma.rs#L233-L244):

```rust
if irq {
    let interrupt = match num {
        0 => InterruptRequest::DMA0,
        // ...
    };
    self.interrupts[0].request |= interrupt;
    self.interrupts[1].request |= interrupt;
}
```

> **Divergence.** The completion IRQ is raised on **both** interrupt
> controllers, not just the one belonging to the CPU that owns the channel. On
> hardware an ARM7 DMA0 completion sets only ARM7's IF. In practice a CPU that
> never enabled `DMA0` in its own IE is unaffected, which is why this has not
> surfaced as a visible bug — but it is a real deviation.

---

## 8.6 The timing gap

The transfer loop accumulates `_cycles_passed`, and then does nothing with it
([dma.rs:229-232](core/src/hw/dma.rs#L229-L232)):

```rust
// TODO: Don't halt CPU if PC is in TCM
// TODO: Add this back - Removed because it broke when CPU synchronization was made looser
// self.clock(_cycles_passed);
```

```text
   Real hardware                    Lunaris today
   ─────────────                    ─────────────
   DMA steals the bus;              DMA completes in zero emulated time;
   the CPU stalls for the           the CPU never stalls.
   duration of the transfer
   (except when running from TCM)
```

This is the largest known accuracy gap in the DMA implementation, and the
comment records _why_ it was removed: re-enabling it under the looser CPU
synchronisation of the current main loop (Chapter 1) broke timing elsewhere.
The underscore prefix on `_cycles_passed` is a deliberate marker that the value
is computed but unused.

melonDS models DMA bus stealing properly in `src/DMA.cpp`, including the TCM
exemption noted in the first TODO.

---

## 8.7 Other divergences

Four `Occasion` variants are decoded but never triggered, each marked in the
enum:

- `StartOfDisplay` — ARM9, start of the visible period
- `MainMemoryDisplay` — ARM9, per-scanline display capture feed
- `GBACartridge` — Slot-2, unused since no Slot-2 device is emulated (Chapter 5)
- `WirelessInterrupt` — ARM7, Wi-Fi-driven DMA (Chapter 18 uses a different path)

Also absent: DMA priority arbitration. When several channels trigger on the
same occasion, `run_dmas_single` runs them in `by_type` insertion order
([dma.rs:257-265](core/src/hw/dma.rs#L257-L265)), whereas hardware always
services the lowest-numbered channel first.

---

[← 7. Interrupts and IPC](07_interrupts_and_ipc.md) | [Next: 9. The 2D Graphics Engines →](09_2d_engine.md)
