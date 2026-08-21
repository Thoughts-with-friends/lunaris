# 12. VRAM Banking and Display Output

The DS has 656 KB of video memory split into nine banks that can be _re-plugged
at runtime_ into nine different roles. This chapter covers that switchboard, and
the LCD timing that drives everything.

GBATEK references:
[VRAM control](https://problemkaputt.de/gbatek.htm#dsmemorycontrolvram) ·
[Video stuff / display timings](https://problemkaputt.de/gbatek.htm#dsvideostuff) ·
[Display capture](https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode) ·
[DISPSTAT / VCOUNT](https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus) ·
[Power control](https://problemkaputt.de/gbatek.htm#dspowercontrol)

---

## 12.1 The nine banks

```text
   bank   size     LCDC address    typical use
   ────   ──────   ─────────────   ───────────────────────────────────────
    A     128 KB   0680_0000h      engine A BG or OBJ, or 3-D textures
    B     128 KB   0682_0000h      engine A BG or OBJ, or 3-D textures
    C     128 KB   0684_0000h      engine B BG, or ARM7 WRAM, or textures
    D     128 KB   0686_0000h      engine B BG, or ARM7 WRAM, or textures
    E      64 KB   0688_0000h      engine A BG, ext. palettes, tex palettes
    F      16 KB   0689_0000h      ext. palettes, tex palettes
    G      16 KB   0689_4000h      ext. palettes, tex palettes
    H      32 KB   0689_8000h      engine B BG, engine B ext. palettes
    I      16 KB   068A_0000h      engine B OBJ / ext. palettes
                                                     total 656 KB
```

([vram.rs:55-69](core/src/hw/gpu/vram.rs#L55-L69))

Each bank has one 8-bit control register, `VRAMCNT_A`..`VRAMCNT_I` at
`4000240h`:

```text
   VRAMCNT_x
     7  6  5  4  3  2  1  0
   ┌──┬──┬──┬──┬──┬──┬──┬──┐
   │EN│  -  │ OFS │  MST   │
   └──┴─────┴─────┴────────┘
     │        │       └── MST: which role this bank plays
     │        └────────── OFS: where within that role it sits
     └─────────────────── enable
```

The legal widths of MST and OFS differ per bank
([vram.rs:554-565](core/src/hw/gpu/vram.rs#L554-L565)):

```rust
impl VRAMCNT {
    const MST_MASKS: [u8; 9] = [0x3, 0x3, 0x7, 0x7, 0x7, 0x7, 0x7, 0x3, 0x3];
    const OFS_MASKS: [u8; 9] = [0x3, 0x3, 0x3, 0x3, 0x0, 0x3, 0x3, 0x0, 0x0];

    pub fn new(index: usize, byte: u8) -> Self {
        VRAMCNT {
            mst: byte & VRAMCNT::MST_MASKS[index],
            offset: byte >> 3 & VRAMCNT::OFS_MASKS[index],
            enabled: byte >> 7 & 0x1 != 0,
            byte,
        }
    }
```

Bank E, H and I have no OFS field at all — masking it to zero rather than
reading garbage is the difference between a working game and a mysterious
half-drawn screen.

---

## 12.2 The switchboard

```text
                        ┌──────────────────────────────────────┐
    bank A ────┐        │  engine A BG        (512 KB region)  │
    bank B ────┤        │  engine A OBJ       (256 KB region)  │
    bank C ────┤        │  engine B BG        (128 KB region)  │
    bank D ────┼──────► │  engine B OBJ       (128 KB region)  │
    bank E ────┤ VRAMCNT│  engine A BG ext. palettes           │
    bank F ────┤        │  engine A OBJ ext. palettes          │
    bank G ────┤        │  engine B BG/OBJ ext. palettes       │
    bank H ────┤        │  3-D textures                        │
    bank I ────┘        │  3-D texture palettes                │
                        │  ARM7 WRAM (banks C/D only)          │
                        │  LCDC direct access                  │
                        └──────────────────────────────────────┘
```

Lunaris models the switchboard as **one `Vec<Vec<Bank>>` per role**, indexed by
16 KB slot ([vram.rs:21-53](core/src/hw/gpu/vram.rs#L21-L53)):

```rust
pub struct VRAM {
    cnts: [VRAMCNT; 9],
    pub(super) banks: [Vec<u8>; 9],
    // Functions
    lcdc_enabled: [bool; 9],
    lcdc: Vec<Vec<Bank>>,
    engine_a_bg: Vec<Vec<Bank>>,
    engine_a_obj: Vec<Vec<Bank>>,
    engine_a_bg_ext_pal: Vec<Vec<Bank>>,
    engine_a_obj_ext_pal: Vec<Vec<Bank>>,
    textures: Vec<Vec<Bank>>,
    textures_pal: Vec<Vec<Bank>>,
    engine_b_bg: Vec<Vec<Bank>>,
    // ...
    arm7_wram: Vec<Vec<Bank>>,
}
```

```text
   engine_a_bg  (32 slots × 16 KB = 512 KB address space)

   slot   banks mapped here
   ────   ────────────────────
    0     [A]           ◄── one bank
    1     [A]
    ...
    8     [B, E]        ◄── TWO banks at the same address: this is legal!
    ...
   31     []            ◄── nothing mapped: reads return 0
```

The **why** of `Vec<Bank>` rather than `Option<Bank>` is the important part.
Nothing stops a game from mapping two banks to overlapping addresses. On real
hardware the results OR together on read and both are written on write, and
Lunaris does exactly that ([vram.rs:485-505](core/src/hw/gpu/vram.rs#L485-L505)):

```rust
fn read_mapping<T: MemoryValue>(banks: &[Vec<u8>], mapping: &Vec<Bank>, addr: usize) -> T {
    let mut value = num::zero();
    for bank in mapping.iter() {
        let addr = addr & (VRAM::BANKS_LEN[*bank as usize] - 1);
        value |= HW::read_mem::<T>(&banks[*bank as usize], addr as u32);
    }
    value
}

fn write_mapping<T: MemoryValue>(
    banks: &mut [Vec<u8>],
    mapping: &Vec<Bank>,
    addr: usize,
    value: T,
) {
    for bank in mapping.iter() {
        let addr = addr & (VRAM::BANKS_LEN[*bank as usize] - 1);
        HW::write_mem(&mut banks[*bank as usize], addr as u32, value);
    }
}
```

An empty `Vec` gives `num::zero()` for free — no special "unmapped" branch.

### Remapping

`write_vram_cnt` is a two-phase operation: _un_-map the old role, then map the
new one ([vram.rs:140-160](core/src/hw/gpu/vram.rs#L140-L160)):

```rust
pub fn write_vram_cnt(&mut self, index: usize, value: u8) {
    let bank = Bank::from_index(index);
    let new_cnt = VRAMCNT::new(index, value);
    crate::diag!(
        "vramcnt",
        "bank {:?}: enabled={} mst={} ofs={} (raw={:#04X})",
        bank, new_cnt.enabled as u8, new_cnt.mst, new_cnt.offset, value
    );

    if self.cnts[index].enabled {
        match (index, self.cnts[index].mst) {
            (index, 0) => {
                assert!(self.lcdc_enabled[index]);
                self.lcdc_enabled[index] = false;
                VRAM::remove_mapping(&mut self.lcdc, bank, VRAM::LCDC_OFFSETS[index], None)
            }
            // ... one arm per (bank, mst) combination
```

```text
   write VRAMCNT_A = 0x81  (enable, MST=1 → engine A BG, OFS=0)

   step 1: was it enabled before?
              yes → remove_mapping(old role, bank A, old offset)
   step 2: is it enabled now?
              yes → add_mapping(engine_a_bg, bank A, 0x00000)
                    which pushes `Bank::A` into slots 0..8
```

`add_mapping` asserts that the bank is not already in the target slot
([vram.rs:507-515](core/src/hw/gpu/vram.rs#L507-L515)) — a cheap guard against
an un-map being skipped, which would otherwise silently double every read.

### Address decoding on the ARM9 side

The role is picked out of the address itself
([vram.rs:302-341](core/src/hw/gpu/vram.rs#L302-L341)):

```rust
pub fn arm9_read<T: MemoryValue>(&self, addr: u32) -> T {
    let index = addr as usize / VRAM::MAPPING_LEN;
    let addr = addr as usize;
    match addr & 0x00E0_0000 {
        VRAM::ENGINE_A_BG_OFFSET => VRAM::read_mapping(
            &self.banks,
            &self.engine_a_bg[index & VRAM::ENGINE_A_BG_MASK],
            addr,
        ),
        VRAM::ENGINE_B_BG_OFFSET => /* ... */,
        VRAM::ENGINE_A_OBJ_OFFSET => /* ... */,
```

```text
   0600_0000h  engine A BG    (ENGINE_A_BG_OFFSET   = 0x00_0000)
   0620_0000h  engine B BG    (ENGINE_B_BG_OFFSET   = 0x20_0000)
   0640_0000h  engine A OBJ   (ENGINE_A_OBJ_OFFSET  = 0x40_0000)
   0660_0000h  engine B OBJ   (ENGINE_B_OBJ_OFFSET  = 0x60_0000)
   0680_0000h  LCDC direct    (per-bank, LCDC_OFFSETS)
```

### The masking rule

The engine-side accessors mask the slot index, and the reason is documented
([vram.rs:396-410](core/src/hw/gpu/vram.rs#L396-L410)):

```rust
/// Reads BG memory of engine A (512 KiB) or engine B (128 KiB).
///
/// The mapping index is masked with the same per-region mask
/// [`VRAM::arm9_read`] applies: `BGCNT.tile_block` is 4 bits wide, so a
/// tile base of up to 0x3_C000 (plus `DISPCNT.char_base` on engine A) can
/// address past the end of the region. Such a read mirrors within the
/// region on real hardware, so it must not panic.
pub fn get_bg<E: EngineType, T: MemoryValue>(&self, addr: usize) -> T {
    let index = addr / VRAM::MAPPING_LEN;
    if E::is_a() {
        VRAM::read_mapping(&self.banks, &self.engine_a_bg[index & VRAM::ENGINE_A_BG_MASK], addr)
    } else {
        VRAM::read_mapping(&self.banks, &self.engine_b_bg[index & VRAM::ENGINE_B_BG_MASK], addr)
    }
}
```

An out-of-range tile base is not a bug in the game — it is a legal
configuration that mirrors. Panicking (or clamping) here would break real
software.

---

## 12.3 LCD timing

```text
   One scanline = 355 dots × 6 master cycles = 2130 cycles

   dot 0                              dot 264                    dot 355
   ├──────────── visible 256 ─────────┼──── H-Blank 99 dots ──────┤
   │                                  │                           │
   StartNextLine event            HBlank event               StartNextLine
   - clear HBLANK flag            - set HBLANK flag
   - VCOUNT++                     - render_line()   ← the WHOLE scanline
   - VBLANK flag at line 192      - HBlank DMA
   - VCOUNT match IRQ             - HBlank IRQ

   One frame = 263 scanlines
   ├──────── visible 0..191 ─────────┼──── V-Blank 192..262 ──────┤
                                     │
                                on_vblank: VBlank DMA, 3-D render,
                                           GXFIFO release
```

Constants ([gpu.rs:69-88](core/src/hw/gpu.rs#L69-L88)):

```rust
    const CYCLES_PER_DOT: usize = 6;
    /// H-Blank begins 8 dots after the last visible pixel (dot 264).
    const HBLANK_DOT: usize = 256 + 8;
    const DOTS_PER_LINE: usize = 355;
    const NUM_LINES: usize = 263;
```

```text
   263 lines × 355 dots × 6 cycles = 560,190 cycles/frame
   33,513,982 / 560,190 = 59.8261 Hz   ← the DS refresh rate
```

The two handlers reschedule each other, so the display is a self-sustaining
two-event loop ([gpu.rs:299-303](core/src/hw/gpu.rs#L299-L303) and
[gpu.rs:358-363](core/src/hw/gpu.rs#L358-L363)):

```rust
    pub fn start_next_line(&mut self, _event: Event) {
        self.scheduler.schedule(
            Event::HBlank,
            HW::on_hblank,
            GPU::HBLANK_DOT * GPU::CYCLES_PER_DOT,
        );
```

```rust
    pub fn on_hblank(&mut self, _event: Event) {
        self.scheduler.schedule(
            Event::StartNextLine,
            HW::start_next_line,
            (GPU::DOTS_PER_LINE - GPU::HBLANK_DOT) * GPU::CYCLES_PER_DOT,
        );
```

This is why the scheduler's queue is never empty (Chapter 6, §6.2).

### DISPSTAT and the VCOUNT-match bug

```text
   DISPSTAT
    15        8 7  6  5  4  3  2  1  0
   ┌───────────┬──┬──┬──┬──┬──┬──┬──┬──┐
   │ LYC 7..0  │L8│ - │VM│HB│VB│VC│HS│VS│
   └───────────┴──┴──┴──┴──┴──┴──┴──┴──┘
                     │  │  │  │  │  └── V-Blank flag        (status)
                     │  │  │  │  └───── H-Blank flag        (status)
                     │  │  │  └──────── VCOUNT match flag   (status)
                     │  │  └─────────── V-Blank IRQ enable
                     │  └────────────── H-Blank IRQ enable
                     └───────────────── VCOUNT match IRQ enable
```

The comment at [gpu.rs:328-337](core/src/hw/gpu.rs#L328-L337) records a real
bug that hit every game:

```rust
        // VCOUNT match: DISPSTAT bit 2 is the status flag and bit 5 is the IRQ
        // enable. Testing the V-Blank enable here (bit 3) made every game that
        // only wanted the V-Blank IRQ also receive a spurious VCOUNT-match IRQ,
        // and starved games that only enabled the VCOUNT IRQ. The status flag
        // is surfaced through `DISPSTAT::read` byte 0.
```

Status flags and IRQ-enable flags live in the same register, one bit apart. Mix
them up and every game gets interrupts it never asked for.

Both CPUs have their own DISPSTAT, hence the helper
([gpu.rs:406-413](core/src/hw/gpu.rs#L406-L413)):

```rust
    fn check_dispstats<F>(&mut self, check: &mut F)
    where
        F: FnMut(&mut DISPSTAT, &mut InterruptController),
    {
        for i in 0..2 {
            check(&mut self.gpu.dispstats[i], &mut self.interrupts[i])
        }
    }
```

---

## 12.4 Display capture

Engine A can write its own output back into a VRAM bank, optionally blended
with what is already there. That is how games do motion blur, radial blur, and
"previous frame" effects ([gpu.rs:168-235](core/src/hw/gpu.rs#L168-L235)):

```text
   source A                     source B
   ┌───────────────────┐        ┌─────────────────────┐
   │ engine A output   │        │ a VRAM block        │
   │       or          │        │       or            │
   │ raw 3-D layer     │        │ main-memory FIFO    │  ← todo!()
   └─────────┬─────────┘        └──────────┬──────────┘
             │                             │
             └──────────┬──────────────────┘
                        ▼
              capture_src: A | B | A+B
                        │
                        │ blend: (A×EVA + B×EVB) / 16, per 5-bit channel
                        ▼
              VRAM write block + offset, one scanline at a time
```

```rust
    pub fn capture(&mut self) {
        let start_addr = self.vcount as usize * GPU::WIDTH;
        let width = self.dispcapcnt.capture_size.width();
        fn get_engine_a_color(engine_a: &Engine2D<EngineA>, _: &Engine3D, index: usize) -> u16 {
            engine_a.pixels()[index]
        }
        fn get_engine3d_color(_: &Engine2D<EngineA>, engine_3d: &Engine3D, index: usize) -> u16 {
            engine_3d.pixel_color(index)
        }
        let src_a: fn(&Engine2D<EngineA>, &Engine3D, usize) -> u16 =
            if self.dispcapcnt.src_a_is_3d_only
                || self.engine_a.dispcnt.display_mode != DisplayMode::Mode0
            {
                get_engine3d_color
            } else {
                get_engine_a_color
            };
```

Capture is armed at line 0 and disarmed at line 192, so a single-frame capture
cannot spill into the next frame
([gpu.rs:306-316](core/src/hw/gpu.rs#L306-L316)):

```rust
        if self.gpu.vcount == 0 {
            self.gpu.capturing = self.gpu.dispcapcnt.enable;
            // ...
        } else if self.gpu.vcount == GPU::HEIGHT as u16 {
            if self.gpu.capturing {
                self.gpu.dispcapcnt.enable = false
            }
```

> **Divergence:** `src_b_fifo` (capture source B = main-memory FIFO) is
> `todo!()` ([gpu.rs:187-189](core/src/hw/gpu.rs#L187-L189)), matching the
> missing display mode 3 in Chapter 9.

---

## 12.5 Power control and the final output

`POWCNT1` at `4000304h` gates each block, and — the visible one — swaps the
engines between the two LCDs:

```text
   POWCNT1
    15                  9  8   3  2  1  0
   ┌──┬──────────────────┬──┬───┬──┬──┬──┐
   │A↑│         -        │EB│ - │3R│3G│EA│LCD│
   └──┴──────────────────┴──┴───┴──┴──┴──┘
     │                    │      │  │  │  └ enable LCDs
     │                    │      │  │  └─── enable engine A
     │                    │      │  └────── enable 3-D geometry
     │                    │      └───────── enable 3-D rendering
     │                    └──────────────── enable engine B
     └───────────────────────────────────── bit 15: engine A drives TOP screen
```

```rust
pub fn get_screens(&self) -> [&Vec<u16>; 2] {
    if self.powcnt1.contains(POWCNT1::TOP_A) {
        [self.engine_a.pixels(), self.engine_b.pixels()]
    } else {
        [self.engine_b.pixels(), self.engine_a.pixels()]
    }
}
```

([gpu.rs:280-286](core/src/hw/gpu.rs#L280-L286))

The full path from bank to LCD, end to end:

```text
   VRAM bank D ──VRAMCNT──► engine_b_bg[slot] ──get_bg──► tile bytes
                                                              │
                                                       decode + palette
                                                              │
                                                      bg_lines[bg_i]
                                                              │
                                                   process_lines (Ch. 9)
                                                              │
                                                   engine_b.pixels[]
                                                              │
                                              POWCNT1::TOP_A ─┴─► screen 0 or 1
                                                              │
                                                     NDS::get_screens()
                                                              │
                                                     frontend texture
```

---

## 12.6 Divergences

- **Main-memory display FIFO** (display mode 3, capture source B FIFO) —
  `todo!()` in both places.
- **VRAM access conflicts** are not modelled: the CPU can write a bank in the
  same cycle the engine reads it with no penalty or corruption.
- **VRAMSTAT** (banks C/D ARM7 allocation status) is implemented, but the ARM7
  mapping only supports the two 128 KB slots.
- **No per-dot rendering.** Every scanline is produced atomically at the H-Blank
  event, which makes mid-scanline VRAM writes invisible (Chapter 9, §9.11).

---

[← 11. The 3-D Rasteriser](11_3d_rasterizer.md) | [Next: 13. The Sound Processing Unit →](13_spu.md)
