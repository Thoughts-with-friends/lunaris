# 10. The 3-D Geometry Engine

The DS 3-D hardware splits cleanly in two: a **geometry engine** that consumes
a command stream and produces a list of screen-space polygons, and a
**rendering engine** that rasterises that list. This chapter covers the first
half; Chapter 11 covers the second.

GBATEK references:
[3-D overview](https://problemkaputt.de/gbatek.htm#ds3doverview) ·
[Geometry commands](https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands) ·
[Matrix stack](https://problemkaputt.de/gbatek.htm#ds3dmatrixstack) ·
[Matrix load/multiply](https://problemkaputt.de/gbatek.htm#ds3dmatrixloadmultiply) ·
[Polygon definitions by vertices](https://problemkaputt.de/gbatek.htm#ds3dpolygondefinitionsbyvertices) ·
[GXSTAT](https://problemkaputt.de/gbatek.htm#ds3dstatus) ·
[3-D I/O map](https://problemkaputt.de/gbatek.htm#ds3diomap)

---

## 10.1 The pipeline

```text
   game code
      │  writes commands
      ▼
   ┌──────────────────────────────────────────────────────────────┐
   │  GXFIFO  (256 entries)                        4000400h       │
   │  or per-command mirror ports                  4000440h+4*ID  │
   └──────────────┬───────────────────────────────────────────────┘
                  │ exec_command
                  ▼
   ┌──────────────────────────────────────────────────────────────┐
   │  Matrix unit         projection / position / direction / tex │
   │  Lighting            4 lights, material colours              │
   │  Vertex assembly     tris, quads, strips                     │
   │  Culling             front/back via clip-space normal        │
   │  Clipping            6 planes, Sutherland–Hodgman            │
   │  Viewport transform  clip space → screen coords + Z depth    │
   └──────────────┬───────────────────────────────────────────────┘
                  │
                  ▼
        vertices: Vec<Vertex>      polygons: Vec<Polygon>
                  │
                  │  SwapBuffers, resolved at V-Blank
                  ▼
        rendering engine (Chapter 11)
```

---

## 10.2 Two ways to submit commands

**Packed FIFO writes** at `4000400h`: one 32-bit word can carry up to four
command IDs, followed by their parameter words
([geometry.rs:302-345](core/src/hw/gpu/engine3d/geometry.rs#L302-L345)):

```text
   Write 1:  [ cmd3 | cmd2 | cmd1 | cmd0 ]   packed command word
   Write 2:  [ cmd0 param 0 ]
   Write 3:  [ cmd0 param 1 ]
   Write 4:  [ cmd1 param 0 ]
   ...
```

```rust
pub fn write_geometry_fifo(&mut self, interrupts: &mut InterruptRequest, value: u32) {
    if self.packed_commands == 0 {
        if value == 0 {
            return;
        }
        self.packed_commands = value;
        self.cur_command = GeometryCommand::from_byte(self.packed_commands as u8);
        self.num_params = self.cur_command.num_params();
        self.params_processed = 0;
        if self.num_params > 0 {
            return;
        }
    } else {
        self.params_processed += 1
    }
    // ... shift packed_commands right by 8 per completed command ...
```

**Mirror ports** at `4000440h + 4*ID`: the address encodes the command
([geometry.rs:347-363](core/src/hw/gpu/engine3d/geometry.rs#L347-L363)):

```rust
pub fn write_geometry_command(
    &mut self,
    interrupts: &mut InterruptRequest,
    addr: u32,
    value: u32,
) {
    let command = GeometryCommand::from_addr(addr & 0xFFF);
    if command != GeometryCommand::Unimplemented {
        self.push_geometry_command(interrupts, command, value);
    }
}
```

Both converge on `push_geometry_command` → `gxfifo.push_back` →
`exec_commands`.

### Multi-parameter commands

A command with N parameters is enqueued N times and accumulated
([geometry.rs:60-72](core/src/hw/gpu/engine3d/geometry.rs#L60-L72)):

```rust
fn exec_command(&mut self, command_entry: GeometryCommandEntry) {
    if self.gxfifo.len() < Engine3D::FIFO_LEN {
        self.gxstat.geometry_engine_busy = false;
        self.bus_stalled = false;
    }
    self.params.push(command_entry.param);
    if self.params.len() < command_entry.command.num_params() {
        if self.params.len() > 1 {
            assert_eq!(self.prev_command, command_entry.command)
        }
        self.prev_command = command_entry.command;
        return;
    }
```

The `assert_eq!` enforces the invariant that a command's parameters are never
interleaved with another command's.

---

## 10.3 The FIFO, the stall, and the DMA trigger

```text
   gxfifo occupancy
   0 ─────────────── 128 ──────────────── 256
   │                  │                     │
   │   DMA refills    │                     │  bus_stalled = true
   │   allowed here   │                     │  (CPU write to GXFIFO blocks)
   │◄─────────────────┤                     │
   should_run_fifo()  │                     │
                      └── IRQ if GXSTAT     │
                          irq mode = LessHalf
```

```rust
/// True when the FIFO has room for a DMA-triggered refill (less than
/// half full, i.e. fewer than 128 entries).
pub fn should_run_fifo(&self) -> bool {
    !self.polygons_submitted && self.gxfifo.len() < Engine3D::FIFO_LEN / 2
}
```

([geometry.rs:17-25](core/src/hw/gpu/engine3d/geometry.rs#L17-L25))

`bus_stalled` is what makes the main loop take its second branch (Chapter 1,
§1.3) ([geometry.rs:47-58](core/src/hw/gpu/engine3d/geometry.rs#L47-L58)):

```rust
pub fn exec_commands(&mut self, interrupts: &mut InterruptRequest) {
    if !self.polygons_submitted {
        while let Some(entry) = self.gxfifo.pop_front() {
            self.exec_command(entry);
            if self.polygons_submitted {
                break;
            }
        }
    }
    self.check_interrupts(interrupts);
    self.bus_stalled = self.gxfifo.len() >= Engine3D::FIFO_LEN;
}
```

`polygons_submitted` is the SwapBuffers latch: once a game issues SwapBuffers,
the geometry engine **halts until the next V-Blank**. Missing this is a classic
freeze bug — the game waits for GXSTAT to go idle, the emulator keeps executing
commands, and the two never agree.

```text
   frame N                                    V-Blank        frame N+1
   ──────────────────────────────────────────────┬──────────────────────
   commands ... SwapBuffers                      │  commands resume
                    │                            │
                    ▼                            ▼
        polygons_submitted = true      render(); polygons_submitted = false
        FIFO execution halts           exec_commands() drains backlog
```

The V-Blank handler does exactly that, and deliberately does _not_ gate on
POWCNT1 ([gpu.rs:378-405](core/src/hw/gpu.rs#L378-L405)):

```rust
    /// The SwapBuffers resolution itself is unconditional: POWCNT1
    /// "Enable 3D Rendering" (bit 2) only gates the rasterizer, not the
    /// geometry engine's halt-until-VBlank behavior. Gating the resolution
    /// on that bit would leave the geometry engine (and GXFIFO) stalled
    /// forever whenever a game toggles rendering off mid-scene.
    pub fn on_vblank(&mut self, _event: Event) {
        self.run_dmas_both(dma::Occasion::VBlank);
        let rendering_enabled = self.gpu.powcnt1.contains(POWCNT1::ENABLE_3D_RENDERING);
        self.gpu.engine3d.render(&self.gpu.vram, rendering_enabled);

        self.gpu.engine3d.exec_commands(&mut self.interrupts[1].request);
        self.check_geometry_command_fifo();
    }
```

---

## 10.4 Matrices

Four matrix modes, three stacks, all 4×4 in 1+19+12 fixed point.

```text
   MTX_MODE 0  Projection    stack depth 1
   MTX_MODE 1  Position      stack depth 31   ┐ pushed/popped together
   MTX_MODE 2  Position+Vec  stack depth 31   ┘
   MTX_MODE 3  Texture       stack depth 1

   Engine3D state:
     cur_proj, cur_pos, cur_vec, cur_tex          ← current matrices
     proj_stack[1], pos_stack[31], vec_stack[31], tex_stack[1]
     proj_stack_sp, pos_vec_stack_sp, tex_stack_sp

   clip_mat = cur_proj × cur_pos     ← recomputed on any change
```

Stack overflow and underflow are **not** errors that stop anything; they set a
status bit ([geometry.rs:81-113](core/src/hw/gpu/engine3d/geometry.rs#L81-L113)):

```rust
            // GBATEK "DS 3D Matrix Stack" (`#ds3dmatrixstack`): overflow or
            // underflow of any matrix stack sets GXSTAT bit 15 (the "mat
            // stack error" flag) and otherwise leaves the geometry engine
            // running - it never halts the emulated machine. The
            // projection and texture stacks hold a single entry each; the
            // position/direction stack holds 31 entries addressed by a
            // 6-bit (0..63, wrapping) pointer.
            MtxPush => match self.mtx_mode {
                MatrixMode::Proj => {
                    if self.proj_stack_sp >= 1 {
                        self.gxstat.mat_stack_error = true;
                    } else {
                        self.proj_stack[0] = self.cur_proj;
                        self.proj_stack_sp += 1;
                    }
                }
```

This is the correct shape for _every_ hardware error condition: a flag the
guest can read, never a host-side panic. An earlier version indexed the stack
directly and could panic on a game that over-pushed.

`MTX_POP` takes a **signed 6-bit** offset, so a game can pop several levels at
once or even "pop" backwards ([geometry.rs:114-117](core/src/hw/gpu/engine3d/geometry.rs#L114-L117)):

```rust
            MtxPop => {
                let offset = param & 0x3F;
                let offset = if offset & 0x20 != 0 { 0xC0 | offset } else { offset } as i8;
```

---

## 10.5 Vertex assembly

`BEGIN_VTXS` selects one of four primitive types; every vertex command appends
to `cur_poly_verts` until enough have arrived
([geometry.rs:489-549](core/src/hw/gpu/engine3d/geometry.rs#L489-L549)):

```rust
fn submit_vertex(&mut self, x: FixedPoint, y: FixedPoint, z: FixedPoint) {
    assert_eq!(self.original_verts.len(), self.cur_poly_verts.len());
    self.prev_pos = [x, y, z];
    let vertex_pos = Vec4::new(x, y, z, FixedPoint::one());
    let clip_coords = self.clip_mat * vertex_pos;
    self.original_verts.push((self.clip_mat, self.prev_pos));

    self.transform_tex_coord(TexCoordTransformationMode::Vertex, None);
    self.cur_poly_verts.push(Vertex {
        clip_coords,
        screen_coords: [0, 0], // Temp - Calculated after clipping
        z_depth: 0,            // Temp - Calculated after clipping
        normalized_w: 0,       // Temp - Calculated after clipping
        color: self.color,
        tex_coord: self.tex_coord,
    });
    match self.vertex_primitive {
        VertexPrimitive::Triangles => {
            if self.cur_poly_verts.len() == 3 {
                self.submit_polygon();
            }
        }
```

```text
   Triangles     v0 v1 v2 │ v3 v4 v5 │ ...        every 3 verts, independent

   Quads         v0 v1 v2 v3 │ v4 v5 v6 v7 │      every 4 verts

   TriangleStrip v0 v1 v2
                    v1 v2 v3        ← reuses last two, winding alternates
                       v2 v3 v4
                 (swap_verts toggles to keep facing consistent)

   QuadStrip     v0 v1 v2 v3
                       v2 v3 v4 v5  ← reuses last two, with a 2↔3 swap
```

The strip handling in Lunaris re-pushes the carried vertices after submitting,
and flips `swap_verts` each triangle — that toggle is why alternate triangles
in a strip do not appear inside-out.

---

## 10.6 Culling

Facing is determined from the clip-space cross product, not from a screen-space
signed area ([geometry.rs:552-587](core/src/hw/gpu/engine3d/geometry.rs#L552-L587)):

```rust
    let mut normal =
        (((a.1 * b.2) - (a.2 * b.1)), ((a.2 * b.0) - (a.0 * b.2)), ((a.0 * b.1) - (a.1 * b.0)));
    while (normal.0 >> 31) ^ (normal.0 >> 63) != 0
        || (normal.1 >> 31) ^ (normal.1 >> 63) != 0
        || (normal.2 >> 31) ^ (normal.2 >> 63) != 0
    {
        normal.0 >>= 4;
        normal.1 >>= 4;
        normal.2 >>= 4;
    }
    let vert = &self.cur_poly_verts[0].clip_coords;
    let dot =
        normal.0 * vert[0].raw64() + normal.1 * vert[1].raw64() + normal.2 * vert[3].raw64();

    let (is_front, should_render) = match dot {
        0 => {
            info!("Not Drawing Line");
            (true, false)
        } // TODO: Line
        _ if dot < 0 => (true, self.polygon_attrs_latch.render_front), // Front
        _ if dot > 0 => (false, self.polygon_attrs_latch.render_back), // Back
        _ => unreachable!(),
    };
```

The `while` loop is a hardware-faithful **normalisation by repeated >>4** until
the components fit in a signed 32-bit range — the DS does the same to keep the
dot product from overflowing. Reproducing it matters, because the shift changes
which polygons land exactly on `dot == 0`.

> **Divergence:** `dot == 0` means the polygon is edge-on and hardware draws it
> as a line. Lunaris discards it (`(true, false)` with a `// TODO: Line`).

---

## 10.7 Clipping

Six planes, applied one axis at a time
([geometry.rs:590-593](core/src/hw/gpu/engine3d/geometry.rs#L590-L593)):

```rust
    // Clip Polygon
    self.clip_plane(2);
    self.clip_plane(1);
    self.clip_plane(0);
```

```text
   View volume in clip space:   -W ≤ X ≤ W,  -W ≤ Y ≤ W,  -W ≤ Z ≤ W

   For each axis, Sutherland–Hodgman against both faces:

      inside            outside
        │                  │
   v0 ──┼───────────────── v1     →  emit v0, emit intersection
        │                  │
   clip_plane(coord_i) walks the vertex ring, calling find_intersection
   for each edge that crosses the plane.
```

Because clipping runs before the viewport transform, screen coordinates are
guaranteed inside the viewport — a fact §10.8 relies on.

---

## 10.8 Viewport transform, and a bug worth studying

[registers.rs:513-545](core/src/hw/gpu/engine3d/registers.rs#L513-L545):

```rust
/// GBATEK "DS 3D Viewport" (`#ds3dviewsvolumesandviewports`):
/// SCREENX = (X+W)*(X2-X1+1)/(2W) + X1,
/// SCREENY = (Y+W)*(Y2-Y1+1)/(2W) + Y1.
///
/// The intermediate products are evaluated in `i64`. `X+W` spans up to
/// `2W`, and W is a 1+19+12 fixed-point value, so `(X+W) * width` can
/// exceed `i32` for large or distant geometry. In release builds that
/// wraps to a negative value, which the final clamp then collapses onto
/// the screen's left/top border: several vertices of one polygon snap to
/// x=0, destroying its shape and making the rasterizer walk a span
/// across most of the scanline. That is the direct cause of the
/// full-width horizontal streaks documented in
/// `docs/design/3d-background-rendering-design.md`.
pub fn screen_coords(&self, clip_coords: &Vec4) -> [u32; 2] {
    let w = clip_coords[3].raw() as i64;
    if w == 0 {
        [0, 0]
    } else {
        let x_offset = clip_coords[0].raw() as i64 + w;
        let y_offset = -(clip_coords[1].raw() as i64) + w;
        let denom = 2 * w;
        let x = x_offset * self.width as i64 / denom + self.x1 as i64;
        let y = y_offset * self.height as i64 / denom + self.y1 as i64;
        [x.clamp(0, GPU::WIDTH as i64 - 1) as u32, y.clamp(0, GPU::HEIGHT as i64 - 1) as u32]
    }
}
```

```text
   The bug, visually:

   correct                              i32 overflow
   ┌────────────────────────┐          ┌────────────────────────┐
   │      ╱▔▔▔╲             │          │▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓│ ← v snapped to x=0,
   │     ╱     ╲            │          │      ╱                 │   span walks the
   │    ╱_______╲           │          │     ╱                  │   whole scanline
   │                        │          │▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓│
   └────────────────────────┘          └────────────────────────┘
```

Note the closing sentence of the doc comment, which is the real lesson: the
clamp is a _safety net_, not the clipping. If the clamp is doing real work,
something upstream is already wrong.

There is a regression test for exactly this
([registers.rs:590-604](core/src/hw/gpu/engine3d/registers.rs#L590-L604)):

```rust
    #[test]
    fn projects_view_volume_corners_to_screen_corners() {
        let viewport = full_screen();
        let w = 0x1000;
        // X = -W maps to the left edge, X = +W to the right edge; Y is
        // flipped because screen Y grows downwards.
        assert_eq!(project(&viewport, -w, w, w), [0, 0]);
        assert_eq!(project(&viewport, 0, 0, w)[0], 128);
    }
```

---

## 10.9 Depth

The 24-bit Z value is computed per vertex from clip-space Z/W. The in-source
comment records a second, subtler bug
([geometry.rs:640-660](core/src/hw/gpu/engine3d/geometry.rs#L640-L660)):

```rust
            // GBATEK "DS 3D Rendering Engine - Depth Buffering"
            // (`#ds3drenderingengine`): the 24-bit Z-buffer value is a
            // monotonic function of clip-space Z/W across the view volume
            // [-W, W]. For vertices sitting almost exactly on the near
            // clip plane (Z very close to -W) the un-clamped formula below
            // evaluates slightly negative before scaling; bitwise-masking
            // that with `& 0xFFFFFF` (as opposed to clamping) wraps it to
            // just under the *far*-plane encoding instead of the *near*
            // one. On real hardware depth is unsigned and the near plane
            // always produces the smallest depth value, so masking here
            // was making close-up geometry (e.g. a camera-filling model
            // during a cutscene) sort as if it were maximally far away,
            // producing the depth-test "torn strip" artifact described in
            // `docs/design/3d-rendering-bugfix-design.md`
```

```text
   masking (wrong)                     clamping (right)
   ───────────────                     ────────────────
   z = -1  →  0xFFFFFF  (far!)         z = -1  →  0x000000  (near)

   Symptom: a model filling the camera sorts behind the background.
```

The fix, with the rounding direction made explicit for negative quotients:

```rust
            let z_scaled = z * 0x4000;
            let z_ratio = if (z_scaled < 0) ^ (w < 0) {
                (z_scaled - (w as i64 - 1)) / w as i64
            } else {
                z_scaled / w as i64
            };

            let mut raw_z_depth = (z_ratio + 0x3FFF) * 0x200;

            if raw_z_depth < 0 {
                raw_z_depth = 0;
            }
```

`normalized_w` is also computed here — a 16-bit magnitude used by the
perspective-correct interpolator in the rasteriser
([geometry.rs:668-672](core/src/hw/gpu/engine3d/geometry.rs#L668-L672)).

---

## 10.10 Lighting and materials

Four directional lights, plus diffuse / ambient / specular / emission material
colours, with an optional 128-entry specular shininess table. Lighting is
evaluated per **vertex** at submission time
([geometry.rs:390-452](core/src/hw/gpu/engine3d/geometry.rs#L390-L452)) — the
DS has no per-pixel lighting.

```text
   NORMAL command
        │
        ├─ transform by the directional (vec) matrix
        ├─ for each enabled light L:
        │     diffuse  = max(0, −L.dir · N) × mat.diffuse × L.colour
        │     ambient  =                      mat.ambient × L.colour
        │     specular = shininess(−(half · N)) × mat.specular × L.colour
        └─ colour = emission + Σ (diffuse + ambient + specular)
```

---

## 10.11 Box test

`BOX_TEST` asks the hardware whether an axis-aligned box intersects the view
volume; games use it to skip whole objects
([geometry.rs:739-775](core/src/hw/gpu/engine3d/geometry.rs#L739-L775)). The
result lands in `GXSTAT.box_test_inside`.

There are unit tests covering the interesting cases
([geometry.rs:892-950](core/src/hw/gpu/engine3d/geometry.rs#L892-L950)):

```rust
    fn box_fully_inside_the_view_volume_is_visible()
    fn box_fully_outside_the_view_volume_is_not_visible()
    fn box_enclosing_the_view_volume_is_visible()
    fn box_straddling_one_face_is_visible()
    fn box_fully_behind_the_view_volume_on_one_axis_is_not_visible()
```

The third one — a box that _contains_ the camera — is the case a naive
"are all 8 corners outside?" test gets wrong, and it manifests as objects
popping out of existence when you walk into them.

---

## 10.12 GXSTAT

[registers.rs:87-102](core/src/hw/gpu/engine3d/registers.rs#L87-L102):

```rust
pub struct GXSTAT {
    pub test_busy: bool, // Box, Pos, Vector Test
    pub box_test_inside: bool,
    pub mat_stack_busy: bool,
    pub mat_stack_error: bool, // Overflow or Underflow
    pub geometry_engine_busy: bool,
    pub command_fifo_irq: CommandFifoIRQ,
}

pub enum CommandFifoIRQ {
    Never = 0,
    LessHalf = 1,
    Empty = 2,
}
```

Reserved value 3 is treated as `Never` rather than panicking
([registers.rs:104-107](core/src/hw/gpu/engine3d/registers.rs#L104-L107)) —
the same "hardware never halts on a bad register value" principle as the matrix
stacks.

---

## 10.13 Divergences

- **No geometry timing.** Commands execute instantly; hardware has per-command
  cycle costs and a vertex/polygon RAM budget. `// TODO: Reject polygon if it
doesn't fit into Vertex RAM or Polygon` marks the missing capacity limit
  ([geometry.rs:611](core/src/hw/gpu/engine3d/geometry.rs#L611)).
- **Line polygons** (`dot == 0`) are dropped.
- **POS_TEST / VEC_TEST** results are computed but `test_busy` is never
  meaningfully timed.
- **Box-test over-culling** was investigated for Pokémon Black/White 2 building
  pop-in; see `docs/design/complete/bw2-3d-building-render-design.md`.

---

[← 9. The 2D Graphics Engines](09_2d_engine.md) | [Next: 11. The 3-D Rasteriser →](11_3d_rasterizer.md)
