# `core/tests` — specification tests that write reports

Every `*.rs` file here mirrors one file in `core/src/` and is compiled **as a
child module of that source file** (see the `#[cfg(test)] #[path = …] mod spec;`
line at the end of each source file). The tests therefore see private fields
and functions exactly like the code they check, and they run as ordinary lib
unit tests — rust-analyzer shows a ▶ Run Test button on every `#[test]`.

```text
core/src/hw/gpu/engine3d/rendering.rs ──(mod spec)──► core/tests/hw/gpu/engine3d/rendering.rs
                                                                │ cargo test
                                                                ▼
                                          core/tests/dist/hw/gpu/engine3d/rendering.md (+ PNGs)
```

Start with [`dist/README.md`](dist/README.md) (generated) — it links every report.

## Running

| How | Command |
| --- | --- |
| everything | `cargo test -p nds-core --lib` |
| one module | `cargo test -p nds-core --lib hw::gpu::engine2d::spec` |
| one report | click ▶ Run Test above `fn report()` in the test file |

Each test **asserts** the GBATEK behaviour and writes its report even when a
documented gap (⚠️) exists; a ❌ means a regression and fails the test.

## Test ROM

Sections marked *real ROM* read a cartridge dump, looked up in this order:

1. environment variable `LUNARIS_TEST_ROM=<path/to/game.nds>`
2. first non-comment line of `core/tests/test_rom.txt` (git-ignored — create it
   locally so the VS Code buttons pick it up)
3. otherwise the synthetic ROM built by `support/rom.rs` (always available;
   it has a valid header, FNT/FAT, overlay table, banner icon and a tiny ARM9
   program that draws a gradient)

Prefer a ≤ 64 MiB title; boot-based tests clone the ROM. `LUNARIS_TEST_FRAMES`
overrides how many frames the "run until something interesting happens" tests
may spend (debug builds run ≈10–18 fps).

## Layout

| Path | Purpose |
| --- | --- |
| `support/md.rs` | Markdown builder: tables, hexdumps, register bit diagrams |
| `support/png.rs` | RGBA canvas: BGR555 blits, lines, triangles, grids, 3×5 font |
| `support/rom.rs` | real/synthetic test ROM |
| `support/boot.rs` | headless boot, run-until-predicate |
| `support/gx.rs` | 3D command driver through the geometry ports |
| `support/spec.rs` | ✅/⚠️/❌ check ledger |
| `dist/` | generated reports (git-ignored) |
