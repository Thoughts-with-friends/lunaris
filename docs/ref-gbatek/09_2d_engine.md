# 9. The 2D Graphics Engines

The DS has two 2-D engines. Each is, roughly, a Game Boy Advance PPU with NDS
extensions bolted on: a 3-D layer, extended palettes, bigger bitmap
backgrounds, and richer OBJ modes.

GBATEK references:
[BG modes / DISPCNT](https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol) ·
[DS OBJs](https://problemkaputt.de/gbatek.htm#dsvideoobjs) ·
[Extended palettes](https://problemkaputt.de/gbatek.htm#dsvideoextendedpalettes) ·
[Windows](https://problemkaputt.de/gbatek.htm#lcdiowindowfeature) ·
[Colour special effects](https://problemkaputt.de/gbatek.htm#lcdiocolorspecialeffects) ·
[BG rotation/scaling](https://problemkaputt.de/gbatek.htm#lcdiobgrotationscaling)

---

## 9.1 Two engines, one implementation

```text
                       Engine2D<E: EngineType>
                                │
              ┌─────────────────┴──────────────────┐
      Engine2D<EngineA>                    Engine2D<EngineB>
      ├ BG0 can display the 3-D layer      ├ no 3-D layer
      ├ display capture (DISPCAPCNT)       ├ no capture
      ├ VRAM display mode (Mode 2)         ├ BG/OBJ only
      ├ larger VRAM allocation             ├ smaller VRAM allocation
      └ palettes at 5000000h               └ palettes at 5000400h

      POWCNT1 bit 15 decides which engine drives which physical LCD.
```

The engine type is a compile-time marker with one method
([gpu.rs:414-429](core/src/hw/gpu.rs#L414-L429)):

```rust
pub trait EngineType {
    fn is_a() -> bool;
}
pub struct EngineA {}
pub struct EngineB {}
```

So `E::is_a()` is a constant at every call site, and Engine B never pays for
3-D-layer or capture logic.

---

## 9.2 The layer model

```text
   For each of the 256 dots of a scanline, up to six sources compete:

   ┌─────────────────────────────────────────────────────────────────┐
   │  OBJ    128 sprites, each with its own priority 0-3             │
   │  BG0    text / 3-D layer (engine A)                             │
   │  BG1    text                                                    │
   │  BG2    text / affine / extended                                │
   │  BG3    text / affine / extended                                │
   │  BD     backdrop = palette entry 0                              │
   └─────────────────────────────────────────────────────────────────┘
             │
             ├─ each BG has a priority 0 (front) .. 3 (back)
             ├─ ties broken by BG number (lower wins)
             └─ an OBJ beats a BG of equal priority
                     │
                     ▼
              window masking (WIN0, WIN1, OBJ window, WINOUT)
                     │
                     ▼
              colour special effects (BLDCNT / BLDALPHA / BLDY)
                     │
                     ▼
              master brightness (MASTER_BRIGHT)
                     │
                     ▼
                 final BGR555 pixel
```

Lunaris renders each layer into its own full-width line buffer first, then
composites ([engine2d.rs:64-76](core/src/hw/gpu/engine2d.rs#L64-L76)):

```rust
    bg_lines: [[u16; GPU::WIDTH]; 4],
    /// Per-dot 5-bit alpha of the 3D layer for the scanline currently in
    /// `bg_lines[0]`, valid only while engine A has BG0 in 3D mode. The 2D
    /// compositor needs it because the 3D layer blends with its own polygon
    /// alpha rather than with BLDALPHA. See [`Engine2D::process_lines`].
    bg0_3d_alphas: [u8; GPU::WIDTH],
    objs_line: [OBJPixel; GPU::WIDTH],
    windows_lines: [[bool; GPU::WIDTH]; 3],
```

Bit 15 of each entry is used as an "opaque" flag — a colour of `0x0000` means
transparent, `0x8000 | colour` means present.

---

## 9.3 Display modes

DISPCNT bits 16-17 pick what the engine outputs at all
([engine2d.rs:138-158](core/src/hw/gpu/engine2d.rs#L138-L158)):

```rust
pub fn render_line(&mut self, engine3d: &Engine3D, vram: &VRAM, vcount: u16) {
    match self.dispcnt.display_mode {
        DisplayMode::Mode0 => {
            for dot_x in 0..GPU::WIDTH {
                self.set_pixel(vcount, dot_x, 0xFFFF);
            }
        }
        DisplayMode::Mode1 => self.render_normal_line(engine3d, vram, vcount),
        DisplayMode::Mode2 => {
            for dot_x in 0..GPU::WIDTH {
                let index = vcount as usize * GPU::WIDTH + dot_x;
                let color = if let Some(bank) = vram.get_lcdc_bank(self.dispcnt.vram_block) {
                    u16::from_le_bytes([bank[index * 2], bank[index * 2 + 1]])
                } else {
                    0
                };
                self.set_pixel(vcount, dot_x, color);
            }
        }
        DisplayMode::Mode3 => todo!(),
    }
}
```

```text
   Mode 0  display off       → white (0xFFFF)
   Mode 1  normal            → BG + OBJ pipeline
   Mode 2  VRAM display      → raw bitmap straight out of an LCDC-mapped bank
   Mode 3  main-memory FIFO  → NOT IMPLEMENTED (todo!)
```

Mode 2 is how games show a full-screen decoded image (movie playback, photo
viewers) without touching the BG hardware.

> **Divergence:** display mode 3 (main-memory display FIFO) panics. It is rare
> — it needs a DMA feeding the display FIFO in mode 4 — but it is a real gap.
> melonDS implements it in `src/GPU2D_Soft.cpp`.

---

## 9.4 Where the pixels come from: ROM → RAM → VRAM → screen

Before the BG modes, it is worth following one tile from the cartridge to the
LCD, because _nothing_ in the 2-D engine reads the ROM. The engine only ever
reads VRAM, palette RAM and OAM. Everything else is the game's job.

```text
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ 1. CARTRIDGE ROM (the .nds file)                                         │
 │                                                                          │
 │    000h ┌───────────────────────────┐                                    │
 │         │ Header (200h bytes)       │ arm9_rom_offset, fat_offset, …     │
 │    200h ├───────────────────────────┤                                    │
 │         │ ARM9 secure area / binary │                                    │
 │         ├───────────────────────────┤                                    │
 │         │ ARM7 binary               │                                    │
 │         ├───────────────────────────┤                                    │
 │         │ FNT  (file name table)    │ directory tree                     │
 │         │ FAT  (file allocation)    │ 8 bytes/file: start + end offset   │
 │         ├───────────────────────────┤                                    │
 │         │ Overlay tables            │                                    │
 │         ├───────────────────────────┤                                    │
 │         │ FILE DATA                 │ ← the graphics live here, usually  │
 │         │  a/0/0/1  (NARC archive)  │   packed into NARC/NCLR/NCGR/NSCR  │
 │         │  a/0/0/2                  │   containers and often LZ77 or     │
 │         │  ...                      │   Huffman compressed               │
 │         └───────────────────────────┘                                    │
 └────────────────────────────────┬─────────────────────────────────────────┘
                                  │  cartridge ROM protocol (Chapter 14)
                                  │  card DMA (Occasion::DSCartridge, Ch. 8)
                                  ▼
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ 2. MAIN RAM  0200_0000h (4 MB)                                           │
 │    the game decompresses / unpacks here                                  │
 └────────────────────────────────┬─────────────────────────────────────────┘
                                  │  DMA or CPU copy (usually during V-Blank)
                                  ▼
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ 3. VIDEO MEMORIES                                                        │
 │                                                                          │
 │  VRAM 0600_0000h   tile pixel data (char blocks) + tilemaps (screen      │
 │                    blocks) + OBJ tiles + extended palettes               │
 │  PAL  0500_0000h   512 BGR555 entries: 000h-1FFh BG, 200h-3FFh OBJ       │
 │  OAM  0700_0000h   128 sprite attribute entries (Ch. 9.6)                │
 └────────────────────────────────┬─────────────────────────────────────────┘
                                  │  Engine2D::render_line, once per scanline
                                  ▼
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ 4. pixels: Vec<u16>  →  NDS::get_screens()  →  frontend texture upload   │
 └──────────────────────────────────────────────────────────────────────────┘
```

Two consequences fall out of this picture, and both matter when debugging:

- **The emulator never parses NARC/NCGR/NCLR/LZ77.** Those are game-side file
  formats. If a tile looks wrong, the bug is in VRAM banking, DMA, or the tile
  decoder — never in "ROM parsing", because there is none.
- **The engine is stateless with respect to the ROM.** The only ROM-derived
  data Lunaris itself reads at boot is the header, to copy the two ARM binaries
  into RAM (Chapter 1, §1.6). Everything visible on screen was placed in VRAM
  by the game.

### Where the engine looks inside VRAM

The two base addresses come from DISPCNT (per engine) plus BGCNT (per BG)
([engine2d.rs:1286-1292](core/src/hw/gpu/engine2d.rs#L1286-L1292)):

```rust
pub(super) fn calc_tile_start_addr(&self, bgcnt: &BGControl) -> usize {
    self.dispcnt.char_base as usize * 0x1_0000 + bgcnt.tile_block() as usize * 0x4000
}

pub(super) fn calc_map_start_addr(&self, bgcnt: &BGControl) -> usize {
    self.dispcnt.screen_base as usize * 0x1_0000 + bgcnt.map_block() as usize * 0x800
}
```

```text
   VRAM as the BG engine sees it (offsets relative to the engine's BG region)

   char_base × 64 KB
        │
        ▼
   ┌─────────────────────────────────────────────────────────────┐
   │ character (tile) blocks — 16 KB each                        │
   │  block 0 │ block 1 │ block 2 │ block 3 │ …                  │
   │  ▲                                                          │
   │  └ bgcnt.tile_block() × 0x4000                              │
   └─────────────────────────────────────────────────────────────┘

   screen_base × 64 KB
        │
        ▼
   ┌─────────────────────────────────────────────────────────────┐
   │ screen (map) blocks — 2 KB each = one 32×32 tilemap          │
   │  blk0 │ blk1 │ blk2 │ blk3 │ blk4 │ …                       │
   │  ▲                                                          │
   │  └ bgcnt.map_block() × 0x800                                │
   └─────────────────────────────────────────────────────────────┘
```

Note the granularity difference: character blocks are 16 KB, screen blocks are
2 KB. A 32×32 tilemap of 16-bit entries is exactly 2048 bytes, which is why the
screen-block unit is what it is.

Engine A's BG region starts at `0600_0000h` and Engine B's at `0620_0000h`, but
neither is a fixed block of silicon — which bank actually answers depends on
VRAMCNT. Chapter 12 covers that mapping; from the 2-D engine's point of view it
is all hidden behind `vram.get_bg::<E, u8>(addr)`.

---

## 9.5 Tile format

A tile is always **8×8 pixels**, stored row-major, either 4 or 8 bits per pixel.

```text
   4bpp tile — 32 bytes                    8bpp tile — 64 bytes
   ─────────────────────                   ────────────────────
   byte 0: [px1|px0]  ← low nibble first   byte 0: px0
   byte 1: [px3|px2]                       byte 1: px1
   byte 2: [px5|px4]                       ...
   byte 3: [px7|px6]   ← row 0 done        byte 7: px7   ← row 0 done
   byte 4..7:  row 1                       byte 8..15:  row 1
   ...                                     ...
   byte 28..31: row 7                      byte 56..63: row 7

   Pixel value 0 is ALWAYS transparent, in both depths.
```

That "low nibble is the left pixel" detail is the single most common tile-decode
bug. Lunaris decodes a whole row at once
([engine2d.rs:1226-1254](core/src/hw/gpu/engine2d.rs#L1226-L1254)):

```rust
pub fn get_colors_from_tile<F: Fn(&VRAM, usize) -> u8>(
    vram: &VRAM,
    get_vram_byte: F,
    addr: usize,
    flip_x: bool,
    flip_y: bool,
    bit_depth: usize,
    tile_y: usize,
    palette_num: usize,
) -> [(usize, usize); 8] {
    let tile_y = if flip_y { 7 - tile_y } else { tile_y };
    let mut colors = [(0, 0); 8];
    let base_addr = addr + tile_y * bit_depth;
    if bit_depth == 8 {
        for (tile_x, color) in colors.iter_mut().enumerate() {
            *color = (0, get_vram_byte(vram, base_addr + tile_x) as usize);
        }
    } else {
        for addr_inc in 0..4 {
            let byte = get_vram_byte(vram, base_addr + addr_inc) as usize;
            colors[2 * addr_inc] = (palette_num, byte & 0xF);
            colors[2 * addr_inc + 1] = (palette_num, byte >> 4 & 0xF);
        }
    }
    if flip_x {
        colors.reverse()
    }
    colors
}
```

`bit_depth` doubles as **bytes per tile row** (4 or 8), which is why
`tile_y * bit_depth` is the row offset — a small piece of arithmetic worth
noticing rather than re-deriving.

Vertical flip is applied to the row index _before_ the fetch; horizontal flip is
a `reverse()` after. Doing X-flip by reversing the decoded row rather than by
reversing bit extraction is both simpler and correct for both depths.

### Address of a tile's pixel data

```text
   tile_addr = tile_start_addr + 8 × bit_depth × tile_num
                                 └── 32 bytes (4bpp) or 64 (8bpp) per tile

   pixel(x, y) = tile_addr + y × bit_depth + x / (8 / bit_depth)
```

([engine2d.rs:1256-1272](core/src/hw/gpu/engine2d.rs#L1256-L1272))

### From colour number to colour

```text
   4bpp:  palette entry = bg_palettes[palette_num × 16 + color_num]
   8bpp:  palette entry = bg_palettes[color_num]            (256-colour)
   8bpp + extended palettes:
          palette entry = extended slot[palette_num][color_num]
                          (a separate VRAM bank, 16 slots × 256 colours)

   BGR555:
    15 14        10 9         5 4         0
   ┌──┬────────────┬────────────┬───────────┐
   │ - │    blue    │   green    │    red    │      5 bits each, 0..31
   └──┴────────────┴────────────┴───────────┘

   Lunaris stores an extra flag in bit 15 internally: 0x8000 = opaque.
```

The BGR555 order — **not** RGB — is another very common porting mistake, and it
shows up as a red/blue swap across the entire screen.

### Fetching a tilemap entry

[engine2d.rs:1118-1150](core/src/hw/gpu/engine2d.rs#L1118-L1150):

```rust
    let addr = map_start_addr + 2 * (map_y * map_size / 8 + map_x);
    let screen_entry = vram.get_bg::<E, u16>(addr) as usize;
    let tile_num = screen_entry & 0x3FF;
    let flip_x = (screen_entry >> 10) & 0x1 != 0;
    let flip_y = (screen_entry >> 11) & 0x1 != 0;
    let original_palette_num = (screen_entry >> 12) & 0xF;
```

```text
   Putting it together, for one 8-pixel span of a text BG scanline:

   ┌──────────┐  map_start_addr + 2×(map_y×32 + map_x)
   │ tilemap  │──────────────────┐
   └──────────┘                  ▼
                        screen_entry = 0xF042
                                 │
                 ┌───────────────┼──────────────┬─────────────┐
                 ▼               ▼              ▼             ▼
            tile_num=0x042   flip_x=0      flip_y=1     palette=0xF
                 │
                 ▼  tile_start_addr + 32 × 0x042
            ┌──────────┐
            │ 8×8 tile │ ── decode row (7 − y%8) ──► 8 colour numbers
            └──────────┘                                 │
                                                         ▼
                                        bg_palettes[0xF × 16 + n]  →  BGR555
                                                         │
                                                         ▼
                                            bg_lines[bg_i][dot_x ..+8]
```

---

## 9.6 BG modes

Mode 1 rendering dispatches BG1–BG3 to text, affine, or extended renderers
according to DISPCNT bits 0-2 ([engine2d.rs:166-258](core/src/hw/gpu/engine2d.rs#L166-L258)):

```text
   BG mode │  BG0        BG1       BG2         BG3
   ────────┼───────────────────────────────────────────
      0    │ text/3D    text      text        text
      1    │ text/3D    text      text        affine
      2    │ text/3D    text      affine      affine
      3    │ text/3D    text      text        extended
      4    │ text/3D    text      affine      extended
      5    │ text/3D    text      extended    extended
      6    │ (large bitmap) — NOT IMPLEMENTED (todo!)
```

The dispatch is written out per mode rather than table-driven, which reads
verbosely but keeps each mode's exact BG set obvious:

```rust
        BGMode::Mode2 => {
            self.render_bg0(engine3d, vram, vcount);
            if self.dispcnt.contains(DISPCNTFlags::DISPLAY_BG1) {
                self.render_text_line(vram, vcount, 1)
            }
            if self.dispcnt.contains(DISPCNTFlags::DISPLAY_BG2) {
                self.render_affine_line(vram, 2, affine_render_fn)
            }
            if self.dispcnt.contains(DISPCNTFlags::DISPLAY_BG3) {
                self.render_affine_line(vram, 3, affine_render_fn)
            }
            self.process_lines(vcount, 0, 3);
        }
```

### Text backgrounds

A text BG is a tilemap: a screen of 16-bit map entries, each selecting a tile
from a character block, with flip and palette bits.

```text
   BG map entry (16 bits)
    15        12 11 10 9              0
   ┌────────────┬──┬──┬────────────────┐
   │ palette    │VF│HF│  tile number   │
   └────────────┴──┴──┴────────────────┘
      (4bpp only) │  └ horizontal flip
                  └─── vertical flip

   Screen sizes and how the four 32×32 maps are laid out:

   size 0  256×256      size 1  512×256      size 2  256×512   size 3 512×512
   ┌──────┐             ┌──────┬──────┐      ┌──────┐          ┌──────┬──────┐
   │  0   │             │  0   │ 800h │      │  0   │          │  0   │ 800h │
   └──────┘             └──────┴──────┘      ├──────┤          ├──────┼──────┤
                                             │ 800h │          │1000h │1800h │
                                             └──────┘          └──────┴──────┘
```

[`render_text_line`](core/src/hw/gpu/engine2d.rs#L946-L1010) implements that
wrap arithmetic explicitly:

```rust
let x_overflowed = (map_x / 32) % 2 == 1;
let y_overflowed = (map_y / 32) % 2 == 1;
let (mut map_start_addr_x_offset, map_start_addr_y_offset) = match bgcnt.screen_size() {
    0 => (0, 0),
    1 => { if x_overflowed { (0x800, 0) } else { (0, 0) } }
    2 => { if y_overflowed { (0, 0x800) } else { (0, 0) } }
    3 => {
        if x_overflowed && y_overflowed { (0x800, 0x800 * 2) }
        else if y_overflowed { (0, 0x800 * 2) }
        else if x_overflowed { (0x800, 0) }
        else { (0, 0) }
    }
    _ => unreachable!(),
};
```

The scanline is then rendered **eight dots at a time** — one tile row per
iteration — with a partial first tile when the horizontal scroll is not a
multiple of 8. Rendering per-tile instead of per-pixel amortises the map fetch
and the palette lookup across eight pixels.

### Affine and extended backgrounds

Affine BGs apply a 2×2 matrix plus a reference point, recomputed per scanline:

```text
   [ x' ]   [ dx  dmx ] [ x ]   [ bgx ]
   [ y' ] = [ dy  dmy ] [ y ] + [ bgy ]

   dx, dmx, dy, dmy : 8.8 fixed point
   bgx, bgy         : 20.8 fixed point, latched at V-Blank
```

The reference points are _latched_, not read live, which is why
[`GPU::start_next_line`](core/src/hw/gpu.rs#L120-L133) re-latches at line 262:

```rust
    if self.vcount == 262 {
        self.engine_a.latch_affine();
        self.engine_b.latch_affine();
    }
```

Writing BGxX/BGxY mid-frame therefore takes effect on the next frame — exactly
as hardware behaves, and the mechanism games use for per-scanline effects by
writing the _delta_ registers instead.

Extended BGs come in three flavours selected by BGCNT bits: 16-bit direct
colour bitmap, 8-bit paletted bitmap, and a 16-bit tilemap with 256-colour
extended palettes.

---

## 9.7 Sprites (OBJ)

128 OAM entries, 8 bytes each: three attribute halfwords plus one shared
affine-parameter halfword.

```text
   OAM entry (attributes 0..2)

   attr0  15 14 13 12 11 10 9  8 7            0
         ┌─────┬──┬──┬─────┬──┬──┬─────────────┐
         │shape│MO│CM│ mode│DS│AF│    Y        │
         └─────┴──┴──┴─────┴──┴──┴─────────────┘
            │    │  │    │   │  └ affine flag
            │    │  │    │   └─── double-size (affine) / disable (normal)
            │    │  │    └─────── 0 normal 1 semi-transparent 2 OBJ window
            │    │  └──────────── 4bpp / 8bpp
            │    └─────────────── mosaic
            └──────────────────── square / wide / tall

   attr1  15 14 13      9 8            0
         ┌─────┬─────────┬──┬───────────┐
         │size │ affine# │VF│HF│   X     │   (VF/HF only when not affine)
         └─────┴─────────┴──┴───────────┘

   attr2  15      12 11 10 9           0
         ┌──────────┬─────┬─────────────┐
         │ palette  │ prio│ tile number │
         └──────────┴─────┴─────────────┘
```

Lunaris parses the whole OAM once per scanline, filters to the sprites that
intersect this line, then sorts by priority
([engine2d.rs:540-572](core/src/hw/gpu/engine2d.rs#L540-L572)):

```rust
fn render_objs_line(&mut self, vram: &VRAM, vcount: u16) {
    let mut oam_parsed = [[0u16; 3]; 0x80];
    let mut affine_params = [[0u16; 4]; 0x20];
    self.oam
        .chunks(8)
        .enumerate() // 1 OAM Entry, 1 Affine Parameter
        .for_each(|(i, chunk)| {
            oam_parsed[i][0] = u16::from_le_bytes([chunk[0], chunk[1]]);
            oam_parsed[i][1] = u16::from_le_bytes([chunk[2], chunk[3]]);
            oam_parsed[i][2] = u16::from_le_bytes([chunk[4], chunk[5]]);
            affine_params[i / 4][i % 4] = u16::from_le_bytes([chunk[6], chunk[7]]);
        });
```

Note the interleaving: affine parameters live in the _fourth halfword_ of four
consecutive OAM entries. Group _n_'s matrix is scattered across entries
`4n..4n+4`, which is why the parse writes `affine_params[i / 4][i % 4]`.

```text
   OAM bytes            0 1  2 3  4 5   6 7
   entry 0            [attr0][attr1][attr2][ PA ]
   entry 1            [attr0][attr1][attr2][ PB ]   ← affine group 0
   entry 2            [attr0][attr1][attr2][ PC ]
   entry 3            [attr0][attr1][attr2][ PD ]
   entry 4            [attr0][attr1][attr2][ PA ]   ← affine group 1
   ...
```

### A per-sprite bug worth reading

The mosaic comment records a fix that is easy to get wrong
([engine2d.rs:585-597](core/src/hw/gpu/engine2d.rs#L585-L597)):

```rust
                // Mosaic applies per sprite, gated on OAM attribute 0 bit 12 -
                // not to every sprite on the line. Quantizing unconditionally
                // made a non-zero OBJ mosaic size (often left over from a
                // transition effect) duplicate rows and columns across every
                // sprite, which shows up as fine horizontal banding on
                // characters.
                let obj_mosaic = obj[0] >> 12 & 0x1 != 0;
```

The vertical wrap case is equally subtle
([engine2d.rs:566-570](core/src/hw/gpu/engine2d.rs#L566-L570)):

```rust
                let obj_y = obj[0] & 0xFF;
                let y_end = obj_y + obj_y_bounds;
                let y = vcount + if y_end > 256 { 256 } else { 0 };
                (obj_y..y_end).contains(&y)
```

A sprite whose Y range crosses 256 wraps to the top of the screen; the
`+ 256` shift is what lets a plain range check handle it.

---

## 9.8 Compositing

[`process_lines`](core/src/hw/gpu/engine2d.rs#L271-L420) is where everything
comes together. It keeps only the **top two layers** per dot, which is all the
blending hardware can see:

```rust
            // Store top 2 layers
            let mut colors = [
                0x8000 | self.bg_palettes[0],
                0x8000 | self.bg_palettes[0], // Default is backdrop color
            ];
            let mut layers = [Layer::BD, Layer::BD];
            let mut priorities = [4, 4];
            let mut i = 0;
            for (bg_i, priority) in bgs.iter() {
                let color = self.bg_lines[*bg_i][dot_x];
                if color & 0x8000 != 0 && enabled[*bg_i] {
                    colors[i] = color;
                    layers[i] = Layer::from(*bg_i);
                    priorities[i] = *priority;
                    if i == 0 { i += 1 } else { break; }
                }
            }
```

Windows are resolved first, and pick which control register applies to this dot
([engine2d.rs:308-320](core/src/hw/gpu/engine2d.rs#L308-L320)):

```rust
            let window_control = if self.windows_lines[0][dot_x] {
                self.win_0_cnt
            } else if self.windows_lines[1][dot_x] {
                self.win_1_cnt
            } else if self.windows_lines[2][dot_x] {
                self.win_obj_cnt
            } else if self.dispcnt.windows_enabled() {
                self.win_out_cnt
            } else {
                WindowControl::all()
            };
```

```text
   Window priority: WIN0 > WIN1 > OBJ window > WINOUT

   ┌──────────────────────────────────────┐
   │ WINOUT (everything not in a window)  │
   │   ┌───────────────┐                  │
   │   │ WIN0          │  ┌────────────┐  │
   │   │      ┌────────┼──┤ WIN1       │  │
   │   │      │ WIN0   │  │            │  │
   │   │      │ wins   │  └────────────┘  │
   │   └──────┴────────┘                  │
   └──────────────────────────────────────┘

   Each window control enables/disables BG0-3, OBJ and colour effects
   independently, which is how games mask a HUD out of a blur effect.
```

### The two blending special cases

Both are documented in-source as regressions that were fixed, and both are
worth stealing:

**Semi-transparent sprites** ([engine2d.rs:369-378](core/src/hw/gpu/engine2d.rs#L369-L378)):

```rust
            // A semi-transparent OBJ is only forced into alpha blending when the
            // pixel underneath it is a 2nd target. Without a 2nd target it falls
            // back to the regular BLDCNT effect, which still requires OBJ to be
            // selected as 1st target; forcing 1st-target here would fade every
            // semi-transparent sprite with whatever brightness value BLDY happens
            // to hold. Matches melonDS `GPU2D_Soft::ApplyColorEffect`, which
            // remaps the sprite flag back to the OBJ target bit before testing
            // BLDCNT.
            let force_alpha = trans_obj && target2_enabled;
```

**The 3-D layer's own alpha** ([engine2d.rs:379-390](core/src/hw/gpu/engine2d.rs#L379-L390)):

```rust
            // The 3D layer carries its own per-pixel alpha. A translucent 3D
            // pixel sitting on a 2nd target always blends with that alpha
            // instead of BLDALPHA, and regardless of whether BG0 is selected as
            // a BLDCNT 1st target; an opaque one is never blended at all.
            // Treating the 3D layer as an ordinary BLDCNT target washed 3D
            // models out to white whenever a game left EVA/EVB at 16/16, since
            // the two layers were then added at full weight and saturated.
            // Matches melonDS `GPU2D_Soft::ColorBlend5`.
            let alpha_3d = if is_3d_layer0 { self.bg0_3d_alphas[dot_x] } else { 0x1F };
            let blend_3d = is_3d_layer0 && alpha_3d < 0x1F && target2_enabled;
```

The 3-D blend uses a different weight scale from ordinary alpha blending —
1/32 units derived from the polygon alpha, not the 1/16 units of BLDALPHA
([engine2d.rs:396-410](core/src/hw/gpu/engine2d.rs#L396-L410)):

```rust
                            // EVA = alpha + 1, EVB = 32 - EVA, in 1/32 units.
                            let eva = alpha_3d as u16 + 1;
                            let evb = 0x20 - eva;
                            let mut new_color = 0;
                            for i in (0..3).rev() {
                                let val1 = colors[0] >> (5 * i) & 0x1F;
                                let val2 = colors[1] >> (5 * i) & 0x1F;
                                let new_val = std::cmp::min(0x1F, (val1 * eva + val2 * evb) >> 5);
                                new_color = new_color << 5 | new_val;
                            }
                            0x8000 | new_color
```

```text
   Colour effects (BLDCNT bits 6-7)
   ────────────────────────────────
   0  none
   1  alpha blend    result = min(31, top×EVA/16 + second×EVB/16)
   2  brightness up  result = top + (31 − top) × EVY/16
   3  brightness dn  result = top − top × EVY/16

   3-D layer override: result = min(31, top×(α+1)/32 + second×(32−α−1)/32)
```

---

## 9.9 How a game draws its UI

There is no "UI layer" on the DS. A menu, a HUD, a text box, a button — all of
it is built out of the same three primitives already described: background
tilemaps, sprites, and windows. Knowing which one a game picked is what lets you
diagnose a UI that renders wrong.

```text
   A typical bottom-screen menu, decomposed

   ┌─────────────────────────────────────────┐
   │ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │  BG3 (prio 3): wallpaper /
   │ ░░┌───────────────────────────────┐░░░░ │       gradient, one tilemap
   │ ░░│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│░░░░ │
   │ ░░│▒  ITEM      x12             ▒│░░░░ │  BG1 (prio 1): window frame +
   │ ░░│▒  POTION    x03             ▒│░░░░ │       text, drawn as tiles
   │ ░░│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│░░░░ │
   │ ░░└───────────────────────────────┘░░░░ │
   │ ░░░░░░░░░░░░ ◆ ░░░░░░░░░░░░░░░░░░░░░░░░ │  OBJ (prio 0): cursor, icons,
   └─────────────────────────────────────────┘       anything that moves
```

### Text is tiles

DS games do not have a font renderer in hardware. Text is drawn by the game's
own code writing tile numbers into a tilemap in VRAM (or by blitting glyph
pixels into a character block), and the 2-D engine simply renders the resulting
tilemap. From the emulator's side, a text box is indistinguishable from any
other background:

```text
   game code                                   engine
   ─────────                                   ──────
   for each character in the string:
     glyph = font_lookup(char)                 (pure game-side work)
     write glyph tile number ──► tilemap  ───► render_text_line reads it back
     write palette number   ──► tilemap
```

This is why a text-rendering bug in an emulator is almost never "text is
broken" — it is a tile decode, a palette selection, or a VRAM bank bug that
happens to be most visible on text, because text has the highest contrast and
the smallest features on screen.

### Sprites are for anything that moves independently

Cursors, dragged items, animated icons, the touch-screen "pen" highlight: these
are OBJs, because a sprite can be repositioned by writing two halfwords to OAM
instead of rewriting a region of tilemap. Games typically:

- keep the cursor as one 16×16 or 32×32 OBJ at priority 0,
- update `attr0.Y` / `attr1.X` once per frame during V-Blank,
- and leave everything static in a BG.

### Windows are for masking

The window feature (§9.8) is how a game shows an effect in one rectangle and
not another — a blurred background behind a _sharp_ menu, a spotlight, a
transition wipe:

```text
   WIN0 = the menu rectangle          WINOUT = everything else
   ┌──────────────────────────────────────────────┐
   │  WINOUT: BG0-3 + OBJ enabled,                │
   │          colour effects ENABLED (fade to black)
   │      ┌────────────────────────────┐          │
   │      │ WIN0: BG1 + OBJ enabled,   │          │
   │      │       colour effects OFF   │  ← menu stays fully lit
   │      └────────────────────────────┘          │
   └──────────────────────────────────────────────┘
```

Because the window control registers select layers _and_ the colour-effect
enable independently, one BLDY brightness ramp plus one window is enough to
build the standard "dim the world, keep the dialogue box readable" look.

### The dual-screen split

The two LCDs are driven by the two engines, and which engine goes where is a
single POWCNT1 bit ([gpu.rs:280-286](core/src/hw/gpu.rs#L280-L286)):

```rust
pub fn get_screens(&self) -> [&Vec<u16>; 2] {
    if self.powcnt1.contains(POWCNT1::TOP_A) {
        [self.engine_a.pixels(), self.engine_b.pixels()]
    } else {
        [self.engine_b.pixels(), self.engine_a.pixels()]
    }
}
```

Games that put the 3-D world on top and the UI on the bottom use Engine A (the
only one with a 3-D layer) for the top screen; games that do the reverse flip
this bit. An emulator that ignores it shows the two screens swapped — a
famously easy bug to spot and an easy one to forget.

---

## 9.10 Built-in diagnostics

Two probe points make "the screen is black" debuggable without a debugger
([engine2d.rs:274-286](core/src/hw/gpu/engine2d.rs#L274-L286)):

```rust
        // Diagnostic D-2: per-layer opacity of one representative scanline,
        // which distinguishes "a layer is black" from "a layer is covered".
        if vcount == 100 && crate::hw::diag::probe("layers") {
            for bg_i in start_line..=end_line {
                let opaque = self.bg_lines[bg_i].iter().filter(|c| *c & 0x8000 != 0).count();
                crate::diag!(
                    "layers",
                    "engine{} bg{bg_i}: enabled={} prio={} opaque={opaque}/256 first={:#06X}",
```

Enabled through the `LUNARIS_DIAG` environment variable (Chapter 20). Building
this kind of probe into the renderer rather than reaching for a debugger is
worth doing early — a black frame otherwise gives you no information at all.

---

## 9.11 Divergences

- **Display mode 3** (main-memory FIFO) — `todo!()`.
- **BG mode 6** (large 512×512 bitmap, Engine A only) — `todo!()`.
- **Mid-scanline register writes** have no effect: an entire scanline is
  rendered at once, at the H-Blank event. Games that change scroll registers
  _within_ a visible line (rare on DS, common on GBA) would look wrong.
- **Per-dot timing** is not modelled; there is no notion of when in the
  scanline a given dot was fetched.

---

[← 8. DMA](08_dma.md) | [Next: 10. The 3-D Geometry Engine →](10_3d_geometry.md)
