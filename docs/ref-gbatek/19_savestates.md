# 19. Savestates

Serialising an emulator sounds like a solved problem — walk the struct, write
the bytes. In practice it is where every hidden assumption in the design comes
due at once: raw pointers, function pointers, host handles, cached indices,
in-flight hardware transactions, and integer widths.

This chapter is the one with the most transferable lessons.

---

## 19.1 The shape

```text
   NDS ──save_state()──► Vec<u8>  (raw emu_utils payload, a few MB)
                             │
                             ▼
              gui/common::savestate::save_to_file
                             │
        ┌────────────────────┴────────────────────┐
        │ LNST header (24 bytes)                  │
        │   magic   "LNST"            4 bytes     │
        │   version u32               4 bytes     │
        │   RomFingerprint           16 bytes     │
        │     game_code u32                       │
        │     header_checksum u16                 │
        │     secure_area_checksum u16            │
        │     rom_len u64                         │
        ├─────────────────────────────────────────┤
        │ zstd-compressed payload (level 3)       │
        └─────────────────────────────────────────┘
                             │
                             ▼
                         .lnst file
```

([gui/common/src/savestate.rs:26-35](gui/common/src/savestate.rs#L26-L35))

The fingerprint lives in the **header**, not the payload, and that placement is
the point ([gui/common/src/savestate.rs:10-13](gui/common/src/savestate.rs#L10-L13)):

```rust
//! Putting the ROM fingerprint in the header (rather than inside the
//! `emu_utils`-serialized payload) lets [`load_from_file`] reject a
//! savestate that belongs to a different ROM *before* any live emulator
//! state is mutated, and without needing to decompress the payload first.
```

```text
   fingerprint in the payload          fingerprint in the header
   ──────────────────────────          ─────────────────────────
   decompress (100 ms)                 read 24 bytes
   start applying state                compare
   discover the mismatch               reject cleanly
   emulator now half-loaded            emulator untouched
```

zstd at level 3 is chosen for speed, not ratio, because this runs on the UI
thread ([gui/common/src/savestate.rs:33-35](gui/common/src/savestate.rs#L33-L35)).

Files without the magic are treated as legacy raw dumps and loaded as-is — a
one-line backwards-compatibility path worth having.

---

## 19.2 The size problem

The first working version produced **137 MB** savestates. The cause was
straightforward once looked at:

```text
   what was in a v1 savestate
   ┌────────────────────────────────────────────┐
   │ cartridge ROM              up to 128 MB    │  ← immutable! re-suppliable!
   │ main RAM                          4 MB     │
   │ VRAM                            656 KB     │
   │ ARM7/ARM9 BIOS                   ~20 KB    │  ← immutable!
   │ everything else                  ~1 MB     │
   └────────────────────────────────────────────┘

   what is in a v2 savestate
   ┌────────────────────────────────────────────┐
   │ main RAM + VRAM + registers + state ~5 MB  │
   │ (zstd → ~2.5 MB on disk)                   │
   └────────────────────────────────────────────┘
```

Two `#[savestate(skip)]` attributes and a fingerprint
([hw.rs:80-86](core/src/hw.rs#L80-L86),
[cartridge.rs:59-68](core/src/hw/cartridge.rs#L59-L68)):

```rust
    /// Not serialized: BIOS images are immutable and re-supplied by the host
    /// at construction time, so shipping ~20 KB of BIOS in every savestate
    /// is pure waste.
    #[savestate(skip)]
    bios7: Vec<u8>,
```

The general rule this expresses:

> **Anything the host re-supplies at construction does not belong in a
> savestate. Store a fingerprint instead and verify it.**

---

## 19.3 What cannot be serialised, and how each is handled

```text
   category                     example                       solution
   ──────────────────────────   ───────────────────────────   ──────────────────
   raw pointers                 arm9_page_table: Vec<*mut u8> skip + rebuild
   function pointers            EventHandler in the scheduler flatten + remap
   host handles                 Audio (cpal stream)           skip
   file-backed state            Backup, firmware Flash         skip + reopen
   derived caches               VRAM bank mapping lists        (they ARE state)
   in-flight transactions       backup chip SPI state          capture explicitly
   intra-frame scratch          attr_buffer                   skip + rebuild
   per-frame reprogrammed       edge_color, fog_table         skip; games rewrite
```

Everything skipped is restored by one hook
([hw.rs:136-152](core/src/hw.rs#L136-L152)):

```rust
    fn post_load_hw<S: emu_utils::ReadSavestate>(&mut self, _save: &mut S) -> Result<(), S::Error> {
        self.scheduler.restore_events(HW::handler_for_event);
        self.init_arm7_page_tables();
        self.init_arm9_page_tables();
        // Clear 3D bus stall so CPUs always run after state load.
        // If the GXFIFO was full at save time, exec_commands at the next VBlank
        // will drain it; clearing the flag here prevents permanent CPU starvation.
        self.gpu.engine3d.bus_stalled = false;
        // Re-evaluate the GXFIFO IRQ condition. `check_interrupts` is normally
        // driven by register writes and command execution, so if the ARM9 was
        // `IntrWait`-ing on GEOMETRY_COMMAND_FIFO at save time, the edge that
        // would wake it up is otherwise lost across a save/load cycle.
        self.gpu.engine3d.check_interrupts(&mut self.interrupts[1].request);
        self.wifi.post_load();
        Ok(())
    }
```

registered on the struct itself ([hw.rs:68-69](core/src/hw.rs#L68-L69)):

```rust
#[derive(emu_utils::Savestate)]
#[load(post = "self.post_load_hw(save)?", in_place_only)]
pub struct HW {
```

### The `Event → handler` remap

Function pointers are meaningless across processes, so the scheduler stores
event _types_ and a table maps them back
([hw.rs:164-176](core/src/hw.rs#L164-L176)):

```rust
    fn handler_for_event(event: &Event) -> EventHandler {
        match event {
            Event::DMA(_, _) => HW::on_dma,
            Event::StartNextLine => HW::start_next_line,
            Event::HBlank => HW::on_hblank,
            Event::VBlank => HW::on_vblank,
            Event::CheckGeometryCommandFIFO => HW::check_geometry_command_fifo_handler,
            Event::TimerOverflow(_, _) => HW::on_timer_overflow,
            Event::ROMWordTransfered(_) => HW::on_rom_word_transfered,
            Event::ROMBlockEnded(_) => HW::on_rom_block_ended,
            Event::GenerateAudioSample => HW::generate_audio_sample,
            Event::StepAudioChannel(_) => HW::step_audio_channel,
            Event::ResetAudioChannel(_) => HW::reset_audio_channel,
            // ...
        }
    }
```

This is the one place that must stay in lockstep with the `Event` enum, and
Rust's exhaustive `match` enforces it at compile time.

---

## 19.4 The three freeze bugs

All three had the same signature: **the emulator keeps running, the game does
not.** Frames advance, audio plays, the CPU executes — and the game sits in a
polling loop forever. That signature always means "the guest is waiting for
something the host forgot to restore."

### (a) The mid-transaction save chip

Covered in Chapter 15, §15.5. A savestate taken while a game was three bytes
into an EEPROM read restored an idle chip.

```text
   fix: BackupProtocolState — capture (mode, address progress, write-enable,
        last value) separately from the memory contents
```

### (b) The lost GXFIFO interrupt edge

If the ARM9 was blocked in `IntrWait` on `GEOMETRY_COMMAND_FIFO` when the state
was taken, the edge that would have woken it was gone on load — because
`check_interrupts` is normally driven by register writes, and no register write
happens during a load.

```text
   fix: call check_interrupts explicitly in post_load_hw
```

### (c) The stuck bus stall

If the GXFIFO was full at save time, `bus_stalled` came back `true` and the main
loop took its stalled branch forever, since nothing would drain the FIFO while
the CPUs were not running.

```text
   fix: clear bus_stalled on load; the next V-Blank drains the FIFO naturally
```

All three fixes are three lines in `post_load_hw`. Finding them took
considerably longer.

---

## 19.5 The `usize` truncation bug

`emu_utils` serialises `usize` as `u32`. Every absolute cycle counter in the
emulator is a `usize` that only grows.

```text
   ARM7 counter @ 33.5 MHz  →  u32 wraps after ~128 s
   ARM9 counter @ 67.0 MHz  →  u32 wraps after  ~64 s

   Symptom: save after two minutes of play, load, and the emulator freezes.
            The scheduler's `cycle` came back as a small number while every
            queued event's fire cycle came back as a small number too — but
            not the SAME small number. Events land in the past or the far
            future; `handle_events` never fires the right one.
```

The fix is an explicit storage type at every counter
([arm.rs:44-54](core/src/arm.rs#L44-L54),
[scheduler.rs:73-96](core/src/hw/scheduler.rs#L73-L96),
[timers.rs:107-117](core/src/hw/timers.rs#L107-L117)):

```rust
    #[store(with = "save.store(&mut (*cycle as u64))?")]
    #[load(
        with = "save.load::<u64>()? as usize",
        with_in_place = "*cycle = save.load::<u64>()? as usize"
    )]
    cycle: usize,
```

And — more valuable than the fix — a way to test it without playing for two
minutes ([nds.rs:200-208](core/src/nds.rs#L200-L208)):

```rust
    /// Test-only: shifts every absolute cycle counter (ARM9/ARM7 CPU cycles,
    /// scheduler cycle, timer start cycles) by `offset`, simulating a long
    /// play session without actually running billions of cycles. ARM9 runs
    /// at 2× the master clock, so its counter is shifted by `2 * offset`.
    #[cfg(test)]
    pub(crate) fn offset_cycles_for_test(&mut self, offset: usize) {
        self.arm9.offset_cycle_for_test(offset * 2);
        self.arm7.offset_cycle_for_test(offset);
        self.hw.offset_cycles_for_test(offset);
    }
```

That helper threads down through `Scheduler::offset_cycle_for_test`,
`Timers::offset_cycles_for_test` and `Timer::offset_start_cycle_for_test` —
four `#[cfg(test)]` functions whose entire purpose is to make an
hours-to-reproduce bug reproducible in microseconds. Building that ladder is
usually cheaper than it looks.

---

## 19.6 The `Vec` in-place load bug

The subtlest of the lot. `emu_utils` offers two load paths:

```text
   Loadable::load()             allocate a new value, return it
   LoadableInPlace::load_in_place()  overwrite an existing value in place
```

For `Vec<T>` and `VecDeque<T>`, `load_in_place` **did not consume the stored
length prefix**.

```text
   savestate bytes:  [len=1024][elem0][elem1]...[elem1023][NEXT FIELD]
                      ^^^^^^^^
   load()            reads len, then 1024 elements   → cursor at NEXT FIELD ✓
   load_in_place()   reads 1024 elements starting AT THE LENGTH PREFIX
                                                     → cursor 4 bytes short ✗

   Every subsequent field in the struct is then read from the wrong offset.
   The damage is silent and total.
```

The workaround is applied at every affected field, always with the same
comment ([ipc.rs:36-41](core/src/hw/ipc.rs#L36-L41)):

```rust
    /// `VecDeque::load_in_place` does not consume the stored length prefix,
    /// so route through `Loadable` instead.
    /// See `docs/design/savestate-and-video-design.md`.
    #[load(with = "save.load()?", with_in_place = "*output7 = save.load()?")]
    output7: VecDeque<u32>,
```

The same annotation appears in [`Engine2D`](core/src/hw/gpu/engine2d.rs#L51-L66),
[`Engine3D`](core/src/hw/gpu/engine3d.rs#L38-L44),
[`VRAM`](core/src/hw/gpu/vram.rs#L18-L23),
[`HW`](core/src/hw.rs#L87-L99) and [`Cartridge`](core/src/hw/cartridge.rs#L79-L81).

> **Lesson:** when a serialisation library gives you two paths that should be
> equivalent and are not, the cost is paid in every struct that uses the wrong
> one, and the symptom appears far from the cause. Documenting the workaround
> at every site — rather than once in a design doc — is what makes it
> survivable.

---

## 19.7 Load-in-place, and why

Note `#[load(in_place_only)]` on `NDS`, `HW`, `Cartridge`, `Engine2D` and `SPI`.
Loading _into an existing emulator_ rather than constructing a new one is what
lets skipped fields (the ROM, the BIOS, the audio device, the open `.sav`) keep
their live values ([nds.rs:26-34](core/src/nds.rs#L26-L34)):

```rust
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), emu_utils::ReadError> {
        use emu_utils::ReadSavestate as _;
        let mut reader = emu_utils::PersistentReadSavestate::new(state)
            .map_err(|_| emu_utils::ReadError::InvalidEnum)?;
        reader.load_into(self)?;
        Ok(())
    }
```

```text
   construct-new                    load-in-place
   ─────────────                    ─────────────
   NDS::from_state(bytes)           nds.load_state(bytes)
        │                                │
   where does the ROM come from?    the ROM is already there
   the audio device?                the audio device is already there
   the open .sav file?              the .sav is already open
```

---

## 19.8 Checklist for your own emulator

```text
   ┌────────────────────────────────────────────────────────────────────┐
   │ □ Is anything in the state a raw pointer?      → skip + rebuild    │
   │ □ A function pointer?                          → tag + remap       │
   │ □ A host resource (audio, file, socket)?       → skip + reopen     │
   │ □ Something the host re-supplies (ROM, BIOS)?  → skip + fingerprint│
   │ □ An in-flight external transaction?           → capture it        │
   │ □ An interrupt condition that is edge-driven?  → re-evaluate on load│
   │ □ A stall/halt flag?                           → verify it can clear│
   │ □ Any absolute counter?                        → check its width   │
   │ □ Can you test "long session" without a long session?              │
   │ □ Does a wrong-ROM load fail BEFORE mutating state?                │
   └────────────────────────────────────────────────────────────────────┘
```

Seven of those ten cost Lunaris a bug each.

---

## 19.9 Divergences

- **Savestates are not portable across builds.** The `emu_utils` layout depends
  on the struct definitions; there is no schema evolution beyond the container
  version byte.
- **`alpha_test_ref`, `edge_color`, `fog_*` and `attr_buffer` are skipped**
  (Chapter 11) so older states still load; games reprogram them per frame.
- **The RTC resumes from the host clock**, not the saved time (Chapter 17).
- **Rewind / frame-history is not implemented** — one state slot at a time.

---

[← 18. Wi-Fi and Local Multiplayer](18_wifi_and_local_mp.md) | [Next: 20. Cheats, Debug Tools and Frontends →](20_cheats_debug_frontend.md)
