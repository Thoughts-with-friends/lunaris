# 11. The 3-D Rasteriser

The rendering engine takes the polygon list produced in Chapter 10 and turns it
into a 256×192 buffer of colours. Lunaris implements it entirely in software,
once per frame, at V-Blank.

GBATEK references:
[3-D overview](https://problemkaputt.de/gbatek.htm#ds3doverview) ·
[Texture formats](https://problemkaputt.de/gbatek.htm#ds3dtextureformats) ·
[Texture blending](https://problemkaputt.de/gbatek.htm#ds3dtextureblending) ·
[Toon / edge / fog / alpha / AA](https://problemkaputt.de/gbatek-ds-3d-toon-edge-fog-alpha-blending-anti-aliasing.htm) ·
[Final 2D output](https://problemkaputt.de/gbatek.htm#ds3dfinal2doutput) ·
[Display control](https://problemkaputt.de/gbatek.htm#ds3ddisplaycontrol)

---

## 11.1 Where polygon and texture data comes from

Like the 2-D engine (Chapter 9, §9.4), the 3-D engine never touches the ROM. It
reads exactly three things, all in VRAM or in registers the game wrote:

```text
 ROM file (.nds)                    game code                  3-D engine
 ─────────────────                  ─────────                  ──────────
 a/1/2/3  model archive             decompress into
   ├ vertex/index data     ──►      main RAM         ──►  written as GXFIFO
   ├ material params                                       geometry commands
   └ textures (NCGR-ish)   ──►      DMA into VRAM    ──►  read by get_tex_color
                                    banks A-D              (texture image slots)
   └ texture palettes      ──►      DMA into VRAM    ──►  read by
                                    banks E/F/G            get_textures_pal
```

```text
   Two very different paths:

   GEOMETRY  ── goes through the CPU/DMA as a *command stream*.
               Nothing persistent; each frame's polygons are re-submitted.
               ROM ─► main RAM ─► GXFIFO ─► vertices/polygons Vec

   TEXTURES  ── are *resident data* in VRAM.
               Uploaded once (or on level load), then sampled every frame.
               ROM ─► main RAM ─► VRAM texture banks ─► sampled per pixel
```

This is why a game with correct geometry but garbled textures is almost always
a VRAM banking problem (Chapter 12), while a game with no geometry at all is a
GXFIFO / DMA problem (Chapters 8, 10).

---

## 11.2 Frame flow

[rendering.rs:62-210](core/src/hw/gpu/engine3d/rendering.rs#L62-L210):

```text
   V-Blank
      │
      ├─ 1. latch frame_params from next_frame_params   (SwapBuffers params
      │                                                   apply to THIS frame)
      ├─ 2. if !rendering_enabled: drop the polygons, release the FIFO, return
      │
      ├─ 3. clear frame_buffer to CLEAR_COLOR / CLEAR_DEPTH
      │     and seed attr_buffer (fog flag, polygon ID)
      │
      ├─ 4. if alpha blending enabled:
      │        partition polygons into opaque (alpha == 0x1F) and translucent
      │        render all opaque
      │        Y-sort translucent back-to-front (unless manual sort requested)
      │        render all translucent
      │     else: render in submission order
      │
      ├─ 5. fog post-pass over the whole buffer (if enabled)
      │
      └─ 6. clear vertices, clear geometry_engine_busy, polygons_submitted = false
                                                        ▲
                                        this is what releases the GXFIFO halt
```

```rust
        if disp3dcnt.alpha_blending {
            let (opaque, mut translucent): (Vec<Polygon>, Vec<Polygon>) =
                self.polygons.drain(..).partition(|polygon| polygon.attrs.alpha == 0x1F);

            for polygon in opaque {
                render(polygon)
            }
            // GBATEK "DS 3D Rendering Engine": unless manual translucent
            // sorting is requested via SwapBuffers, translucent polygons
            // are Y-sorted back-to-front before compositing so overlapping
            // transparent surfaces blend in the correct order.
            if !self.frame_params.manual_sort_translucent {
                translucent.sort_by_key(|polygon| polygon.y_bounds.0);
            }
            for polygon in translucent {
                render(polygon)
            }
        }
```

Step 2 is load-bearing and easy to get wrong — see the comment at
[rendering.rs:54-59](core/src/hw/gpu/engine3d/rendering.rs#L54-L59): POWCNT1's
"enable 3-D rendering" bit gates _only_ the rasteriser. If it also gated the
SwapBuffers resolution, a game that toggles rendering off mid-scene would stall
the GXFIFO forever and freeze the CPU.

---

## 11.3 Scan conversion

The DS rasteriser walks a polygon by tracking a **left edge** and a **right
edge** simultaneously, one scanline at a time.

```text
                start_vert (topmost, leftmost on ties)
                      ●
                     ╱ ╲
       left edge   ╱     ╲   right edge
                 ╱         ╲
        y ──►   ●───────────●   ← for each y: x_start .. x_end
                 ╲         ╱         interpolate colour, S, T, depth
                   ╲     ╱
                     ╲ ╱
                      ●  end_vert (bottommost, rightmost on ties)

   When an edge reaches its end vertex, advance to the next vertex in the
   ring — direction depends on the polygon's winding.
```

Finding the extremes, with the tie-break on X
([rendering.rs:315-334](core/src/hw/gpu/engine3d/rendering.rs#L315-L334)):

```rust
        for (i, vert) in vertices.iter().enumerate() {
            if vert.screen_coords[1] < vertices[start_vert].screen_coords[1] {
                start_vert = i;
            } else if vert.screen_coords[1] == vertices[start_vert].screen_coords[1]
                && vert.screen_coords[0] < vertices[start_vert].screen_coords[0]
            {
                start_vert = i;
            }
            // ... same for end_vert, with > and >
        }
```

Winding decides which walk direction is "left"
([rendering.rs:340-353](core/src/hw/gpu/engine3d/rendering.rs#L340-L353)):

```rust
        // Winding direction picks which walk direction is the "left" edge;
        // plain function pointers avoid a per-polygon heap allocation that
        // a `Box<dyn Fn>` pair would otherwise incur (see
        // `docs/design/3d-rendering-bugfix-design.md` §5.2).
        let is_front = polygon.is_front;
        let next_left = |cur| if is_front { next(cur) } else { prev(cur) };
        let next_right = |cur| if is_front { prev(cur) } else { next(cur) };
```

Note the closure-vs-`Box<dyn Fn>` remark: this function runs per polygon,
thousands of times per frame, so an allocation here is measurable.

The Y range is clamped a second time even though the viewport transform already
clamped it ([rendering.rs:355-361](core/src/hw/gpu/engine3d/rendering.rs#L355-L361)):

```rust
        // Defense-in-depth: screen coordinates are clamped at the viewport
        // transform (see `Viewport::screen_coords`), but clamp the scanline
        // range here too so a future regression can't index past the
        // 256x192 frame buffer.
        let y_start = vertices[start_vert].screen_coords[1].min(GPU::HEIGHT as u32);
        let y_end = vertices[end_vert].screen_coords[1].min(GPU::HEIGHT as u32);
```

---

## 11.4 Interpolation: three different slopes

Not everything interpolates the same way, which is the crux of correct 3-D
rasterisation.

```text
   quantity   interpolation             type
   ─────────  ────────────────────────  ─────────────────
   X          linear in Y               VertexSlope
   depth      linear in X               Slope
   colour     perspective-corrected     ColorSlope   (needs W)
   S, T       perspective-corrected     PerspectiveSlope (needs W)
   W          linear in X               (used as the correction factor)
```

```text
   Why texture coordinates need W:

   linear interpolation in screen space          perspective-correct
   ┌────────────────────────┐                    ┌────────────────────────┐
   │ ▓▒░ ▓▒░ ▓▒░ ▓▒░ ▓▒░    │  a floor stretches │ ▓▓▒░ ▓▒░ ▓░ ▒ ░        │
   │ evenly spaced texels   │  wrongly into the  │ texels compress toward  │
   │                        │  distance          │ the horizon             │
   └────────────────────────┘                    └────────────────────────┘

   correct  =  (S0/W0 · (1−t) + S1/W1 · t) / (1/W0 · (1−t) + 1/W1 · t)
```

`normalized_w` — the 16-bit W magnitude computed back in Chapter 10, §10.9 — is
what feeds those denominators
([rendering.rs:394-422](core/src/hw/gpu/engine3d/rendering.rs#L394-L422)):

```rust
            let mut s = PerspectiveSlope::new(
                left_slope.next_s(),
                right_slope.next_s(),
                num_steps,
                w_start,
                w_end,
            );
```

---

## 11.5 The per-pixel loop

[rendering.rs:425-469](core/src/hw/gpu/engine3d/rendering.rs#L425-L469):

```rust
            for x in x_start..x_end {
                let y = y as usize;
                let depth_val = depth.next() as u32;
                let index = y * GPU::WIDTH + x;
                let pixel = &mut frame_buffer[index];
                let attr = &mut attr_buffer[index];

                let vert_color = FrameBufferColor::new5(color.next(), polygon.attrs.alpha);
                let fb_color = &pixel.color;
                let poly_color =
                    blend(polygon, vert_color, s.next() as i32 >> 4, t.next() as i32 >> 4);
                if poly_color.a5() <= alpha_test_ref {
                    // Fragment failed the alpha test (or is fully transparent),
                    // so neither color, depth, nor attributes are written.
                } else if disp3dcnt.alpha_blending
                    && fb_color.a5() != 0
                    && poly_color.a5() != 0x1F
                    && depth_test(pixel.depth, depth_val)
                {
                    // ... translucent blend path ...
                } else if depth_test(pixel.depth, depth_val) {
                    pixel.color = poly_color;
                    pixel.depth = depth_val;
                    attr.fog = polygon.attrs.fog_enable;
                    attr.opaque_id = polygon.attrs.polygon_id;
                }
            }
```

```text
   Fragment decision tree

   fragment
      │
      ├─ alpha ≤ ALPHA_TEST_REF ?  ──yes──► discard entirely
      │                                     (no colour, no depth, no attrs)
      ├─ translucent AND destination visible AND depth passes ?
      │       ──yes──► blend colour
      │                depth written ONLY if polygon.set_depth_translucent
      │                attr.fog &= polygon fog flag   ← AND, never OR
      │                attr.translucent_id = polygon ID
      │
      └─ depth passes ?  ──yes──► write colour, depth, fog flag, opaque ID
```

The translucent fog rule is subtle and documented in place
([rendering.rs:457-463](core/src/hw/gpu/engine3d/rendering.rs#L457-L463)):

```rust
                    // GBATEK "DS 3D Toon, Edge, Fog, Alpha Blending,
                    // Anti-aliasing"; melonDS `PlotTranslucentPixel`: a
                    // translucent write can only *clear* the destination's
                    // fog flag, never set it (the polygon's own fog flag is
                    // ANDed in, not OR'd).
                    attr.fog &= polygon.attrs.fog_enable;
```

### Depth test modes

```text
   POLYGON_ATTR bit 14 = 0  →  "less than"  (normal)
   POLYGON_ATTR bit 14 = 1  →  "equal"      (with a tolerance window)

   The "equal" mode is what games use to draw decals — shadows, tyre marks,
   text on a surface — coplanar with existing geometry without Z-fighting.
```

([rendering.rs:687-706](core/src/hw/gpu/engine3d/rendering.rs#L687-L706))

---

## 11.6 Textures

### Formats

Seven, all sampled by [`get_tex_color`](core/src/hw/gpu/engine3d/rendering.rs#L474-L642):

```text
   format          bits/texel  notes
   ──────────────  ──────────  ─────────────────────────────────────────
   NoTexture            –      polygon uses vertex colour only
   A3I5                 8      5-bit palette index + 3-bit alpha
   Palette4             2      4 colours, entry 0 optionally transparent
   Palette16            4      16 colours
   Palette256           8      256 colours
   Compressed4x4        2      4×4 block compression + index table
   A5I3                 8      3-bit palette index + 5-bit alpha
   Direct              16      raw BGR555 + 1-bit alpha
```

The A3I5 alpha expansion is a nice detail
([rendering.rs:519-527](core/src/hw/gpu/engine3d/rendering.rs#L519-L527)):

```rust
            TextureFormat::A3I5 => Some({
                let byte = vram.get_textures::<u8>(vram_offset + texel);
                let palette_color = byte & 0x1F;
                let alpha = byte >> 5 & 0x7;
                let color = Color::from(
                    vram.get_textures_pal::<u16>(pal_offset + 2 * palette_color as usize),
                );
                FrameBufferColor::new5(color, alpha * 4 + alpha / 2)
            }),
```

`alpha * 4 + alpha / 2` maps 0..7 onto 0..31 with the right endpoints — a
3-bit-to-5-bit expansion, not a plain shift.

### Wrapping, mirroring, clamping

```text
   TEXIMAGE_PARAM bits 16-18 per axis

   repeat = 0 (clamp)          repeat = 1            repeat + flip
   ┌────┬────┬────┐            ┌────┬────┬────┐      ┌────┬────┬────┐
   │ABC │CCC │CCC │            │ABC │ABC │ABC │      │ABC │CBA │ABC │
   └────┴────┴────┘            └────┴────┴────┘      └────┴────┴────┘
```

[rendering.rs:481-513](core/src/hw/gpu/engine3d/rendering.rs#L481-L513), with an
off-by-one fix recorded in the comments:

```rust
        // Clamped (non-repeating) mode: the valid texel range is
        // `0 ..= size - 1`, so a coordinate equal to the texture dimension
        // must already clamp to the last texel instead of being used as an
        // index one column past the end of the texture.
        } else if s < 0 {
            0
        } else if s as u32 >= size.0 {
            mask.0
        } else {
            s as u32
        } as usize;
```

Mirroring is a XOR with the mask when the tile index is odd — no branchy
reversal needed:

```rust
            if polygon.tex_params.flip_s && (original_s >> size_shift.0) % 2 == 1 {
                s ^ mask
            } else {
                s
            }
```

---

## 11.7 Texture blending modes

The `blend` closure in `render` handles all four
([rendering.rs:104-163](core/src/hw/gpu/engine3d/rendering.rs#L104-L163)):

```text
   Modulation   result = (tex + 1) × (vert + 1) − 1) / 64     per component
                the default; texture tints by vertex colour

   Decal        result = (tex × At + vert × (63 − At)) / 64
                texel alpha selects between texel and vertex colour;
                vertex alpha passes through untouched

   Toon         vertex red channel indexes a 32-entry TOON_TABLE,
                then modulation-blends — cel shading

   Highlight    modulation, then the vertex component added on top,
                CLAMPED to 63

   Shadow       not rasterised (see §11.9)
```

The highlight clamp carries a bug story worth repeating
([rendering.rs:118-127](core/src/hw/gpu/engine3d/rendering.rs#L118-L127)):

```rust
                // GBATEK "DS 3D Texture Blending" (`#ds3dtextureblending`),
                // highlight shading: the modulated value has the vertex
                // component added on top and the sum is *clamped* to the 6-bit
                // maximum. Using `max` here instead of `min` forced every
                // highlight-shaded surface to at least 63 in all three
                // channels, i.e. flat white.
                PolygonMode::ToonHighlight if disp3dcnt.highlight_shading => Self::blend_tex(
                    tex_color,
                    vert_color,
                    |val1, val2| std::cmp::min(modulation_blend(val1, val2) + val2, 0x3F),
                    modulation_blend,
                ),
```

`min` vs `max` — one character, and every highlighted surface turns white.

---

## 11.8 Fog

A whole-frame post-pass rather than a per-fragment operation
([rendering.rs:227-297](core/src/hw/gpu/engine3d/rendering.rs#L227-L297)):

```text
   depth
     │
     │                         FOG_TABLE[31] ──── density ceiling
     │                    ╱▔▔▔
     │              ╱▔▔▔▔▔
     │        ╱▔▔▔▔▔
     │  ╱▔▔▔▔▔ FOG_TABLE[0]
     └──┬──────────────────────────────────►
     FOG_OFFSET      ◄─ range set by DISP3DCNT fog depth shift ─►

   for each pixel with attr.fog set:
       density = interpolate(FOG_TABLE, depth)
       colour  = mix(colour, FOG_COLOR, density)
```

The fog flag comes from three places, and getting all three right is what makes
fog look correct at object edges:

1. an opaque polygon's `POLYGON_ATTR` fog bit (written on a depth pass),
2. `CLEAR_COLOR` bit 15, seeding pixels the rear plane still shows through
   ([rendering.rs:88-104](core/src/hw/gpu/engine3d/rendering.rs#L88-L104)),
3. a translucent write, which may only _clear_ it (§11.5).

---

## 11.9 Output to the 2-D engine

The rasteriser's buffer becomes Engine A's BG0
([rendering.rs:31-42](core/src/hw/gpu/engine3d/rendering.rs#L31-L42)):

```rust
pub fn copy_line(
    &self,
    vcount: u16,
    line: &mut [u16; GPU::WIDTH],
    alphas: &mut [u8; GPU::WIDTH],
) {
    for (i, (pixel, alpha)) in line.iter_mut().zip(alphas.iter_mut()).enumerate() {
        let fb = &self.frame_buffer[vcount as usize * GPU::WIDTH + i].color;
        *alpha = fb.a5();
        *pixel = fb.as_u16();
    }
}
```

The per-pixel alpha is exported alongside the colour, because the 2-D
compositor blends the 3-D layer with _that_ alpha rather than with BLDALPHA —
Chapter 9, §9.8.

```text
   Engine3D.frame_buffer                Engine2D<EngineA>
   ┌──────────────────┐    copy_line    ┌──────────────────┐
   │ 256×192 RGBA     │ ──────────────► │ bg_lines[0]      │  ← BG0
   │ + depth + attrs  │   (per scanline)│ bg0_3d_alphas[]  │
   └──────────────────┘                 └──────────────────┘
                                                 │
                                                 ▼
                                         priority-sorted with
                                         BG1-3, OBJ, backdrop
```

Note the direction of the coupling: the 3-D engine renders a **whole frame** at
V-Blank, but the 2-D engine consumes it **one scanline at a time** during the
next frame's visible period. That is the same split real hardware uses.

---

## 11.10 Divergences

- **No edge marking.** `edge_color` is stored (and skipped by the savestate)
  but never applied; DISP3DCNT bit 5 has no effect.
- **No anti-aliasing.** DISP3DCNT bit 4 is ignored.
- **Shadow polygons are skipped entirely**
  ([rendering.rs:311-313](core/src/hw/gpu/engine3d/rendering.rs#L311-L313)):
  `if polygon.attrs.mode == PolygonMode::Shadow { return; }`. Shadow volumes
  need a stencil pass Lunaris does not implement, so shadows are simply absent
  rather than wrong.
- **No rear-plane bitmap.** `clrimage_offset` is stored but DISP3DCNT bit 14 is
  not honoured; the rear plane is always a flat CLEAR_COLOR.
- **No rendering timing.** Real hardware rasterises 48 scanlines ahead of the
  beam and can run out of time; Lunaris renders the whole frame instantly at
  V-Blank.
- **Interpolation uses `f32`**, not the hardware's fixed-point interpolators, so
  individual pixels can differ by one from a real DS.

---

[← 10. The 3-D Geometry Engine](10_3d_geometry.md) | [Next: 12. VRAM Banking and Display Output →](12_vram_and_display.md)
