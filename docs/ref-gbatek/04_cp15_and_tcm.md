# 4. CP15, TCM and the Protection Unit

The ARM9 has a system-control coprocessor, CP15, reached with the `MCR` / `MRC`
instructions. It controls the tightly-coupled memories, the caches, the
protection unit, the exception vector base — and, importantly for emulation,
the "wait for interrupt" halt.

GBATEK references:
[CP15 overview](https://problemkaputt.de/gbatek.htm#armcp15overview) ·
[Control register](https://problemkaputt.de/gbatek.htm#armcp15controlregister) ·
[Cache and TCM](https://problemkaputt.de/gbatek.htm#dsmemorycontrolcacheandtcm) ·
[Protection unit](https://problemkaputt.de/gbatek.htm#armcp15protectionunitpu)

The ARM7 has no coprocessor at all — everything in this chapter is ARM9-only.

---

## 4.1 How CP15 is addressed

```text
   MCR p15, opcode1, Rd, Cn, Cm, opcode2      (write)
   MRC p15, opcode1, Rd, Cn, Cm, opcode2      (read)
                            │   │      │
                            │   │      └── p   ("opcode2")
                            │   └───────── m   (secondary register)
                            └───────────── n   (primary register)
```

Lunaris dispatches on exactly that `(n, m, p)` triple
([mem/cp15.rs:55-79](core/src/hw/mem/cp15.rs#L55-L79)):

```rust
pub fn read(&self, n: u32, m: u32, p: u32) -> u32 {
    info!("Reading from C{}, C{}, {}", n, m, p);
    match n {
        0 if (m, p) == (0, 1) => 0x0F0D2112, // Cache Type Register
        1 => self.read_control_reg(m, p),
        5 => self.read_ap_regions(m, p),
        6 => self.read_pu_regions(m, p),
        9 => self.read_cache_control(m, p),
        _ => todo!(),
    }
}

pub fn write(&mut self, n: u32, m: u32, p: u32, value: u32) {
    info!("Writing 0b{:b} to C{}, C{}, {}", value, n, m, p);
    match n {
        1 => self.write_control_reg(m, p, value),
        2 => self.write_cachability(m, p, value),
        3 => self.write_cache_write_bufferability(m, p, value),
        5 => self.write_ap_regions(m, p, value),
        6 => self.write_pu_regions(m, p, value),
        7 => self.write_cache_command(m, p, value),
        9 => self.write_cache_control(m, p, value),
        _ => todo!(),
    }
}
```

Register map, and what Lunaris does with each:

```text
  Cn   name                          Lunaris behaviour
  ───  ────────────────────────────  ─────────────────────────────────────────
  C0   ID / cache type               returns constant 0x0F0D2112
  C1   control register              fully modelled (TCM enable, vector base)
  C2   cachability bits              accepted, logged, ignored
  C3   write-bufferability           accepted, logged, ignored
  C5   access permission regions     stored, never enforced
  C6   protection unit regions 0-7   stored, never enforced
  C7   cache commands                (0,4) = WAIT_FOR_IRQ → real halt;
                                     everything else logged and ignored
  C9   cache lockdown + TCM control  TCM base/size fully modelled
```

---

## 4.2 The control register

[mem/cp15.rs:229-248](core/src/hw/mem/cp15.rs#L229-L248):

```rust
bitflags! {
    struct Control: u32 {
        const ITCM_WRITE_ONLY = 1 << 19;
        const ITCM_ENABLE = 1 << 18;
        const DTCM_WRITE_ONLY = 1 << 17;
        const DTCM_ENABLE = 1 << 16;
        const PRE_ARMV5 = 1 << 15;
        const CACHE_REPLACEMENT = 1 << 14;
        const INTERRUPT_BASE = 1 << 13;
        const INSTR_CACHE_ENABLE = 1 << 12;
        const BRANCH_PREDICTION = 1 << 11;
        const BIG_ENDIAN = 1 << 7;
        const LATE_ABORT = 1 << 6;
        const ADDRESS_FAULTS_32 = 1 << 5;
        const EXCEPTION_HANDLING_32 = 1 << 4;
        const WRITE_BUFFER_ENABLE = 1 << 3;
        const DATA_UNIFIED_CACHE_ENABLE = 1 << 2;
        const ALIGNMENT_FAULT_CHECK = 1 << 1;
        const PU_ENABLE = 1 << 0;
    }
}
```

```text
  CP15 C1,C0,0 — Control Register
   31          20 19 18 17 16 15 14 13 12 11 ... 7 6 5 4 3 2 1 0
  ┌──────────────┬──┬──┬──┬──┬──┬──┬──┬──┬──┬─────┬─┬─┬─┬─┬─┬─┬─┐
  │   reserved   │IW│IE│DW│DE│V5│CR│VB│IC│BP│ ... │B│L│A│E│W│C│A│P│
  └──────────────┴──┴──┴──┴──┴──┴──┴──┴──┴──┴─────┴─┴─┴─┴─┴─┴─┴─┘
                   │  │  │  │              │
        ITCM write─┘  │  │  │              └─ VB: vector base
        ITCM enable───┘  │  │                    0 → 0000_0000h
        DTCM write───────┘  │                    1 → FFFF_0000h
        DTCM enable─────────┘
```

Only two of those bits actually change emulator behaviour today. `INTERRUPT_BASE`
selects the exception vector base used by `handle_irq` (Chapter 3)
([mem/cp15.rs:115-124](core/src/hw/mem/cp15.rs#L115-L124)):

```rust
fn write_control_reg(&mut self, m: u32, p: u32, value: u32) {
    // ...
    self.control.bits = value & Control::MASK | Control::ALWAYS_SET;
    self.interrupt_base =
        if self.control.contains(Control::INTERRUPT_BASE) { 0xFFFF_0000 } else { 0x0000_0000 };
}
```

`MASK` drops the bits the ARM946E-S does not implement; `ALWAYS_SET` forces
bits 6/5/4/3 high, matching hardware, which reads them back as 1 regardless of
what was written. Reset value is `0x52078`
([mem/cp15.rs:252-270](core/src/hw/mem/cp15.rs#L252-L270)).

---

## 4.3 Tightly Coupled Memory (TCM)

TCM is on-die SRAM with single-cycle access, _mapped over_ the normal address
space. The ARM9 has two: 32 KB ITCM for instructions and 16 KB DTCM for data.

```text
   ARM9 virtual address space with TCM active
   ────────────────────────────────────────────────────────────────
   0000_0000  ┌──────────────────────┐  ITCM — base is always 0,
              │  ITCM   32 KB        │        size configurable, mirrored
   0000_8000  │  (mirrors up to      │        across its "virtual size"
              │   virtual_size)      │
              ├──────────────────────┤
   0200_0000  │  Main RAM  4 MB      │
              ├──────────────────────┤
   0280_0000  │      ...             │
   0280_3000  ┌──────────────────────┐  DTCM — base *and* size configurable
              │  DTCM   16 KB        │        (default 0080_3000h in Lunaris)
   0280_7000  └──────────────────────┘
              ...
   FFFF_0000  ┌──────────────────────┐
              │  ARM9 BIOS           │
              └──────────────────────┘

   Priority when regions overlap (highest first):
        ITCM  >  DTCM  >  everything else
```

The control word packs base and size together
([mem/cp15.rs:198-226](core/src/hw/mem/cp15.rs#L198-L226)):

```rust
struct TCMControl {
    pub base: u32,
    pub virtual_size: u32,
    virtual_size_shift: u32,
}

impl TCMControl {
    pub fn read(&self) -> u32 {
        self.base | self.virtual_size_shift << 1
    }

    pub fn write(&mut self, value: u32) {
        self.base = value & !0xFFF;
        self.virtual_size_shift = value >> 1 & 0x1F;
        assert!((3..=23).contains(&self.virtual_size_shift));
        self.virtual_size = 0x200 << self.virtual_size_shift;
    }
}
```

```text
  CP15 C9,C1,x — TCM base/size
   31                     12 11    6 5     1 0
  ┌─────────────────────────┬───────┬───────┬─┐
  │        base address     │   -   │ size  │-│    virtual_size = 512 << size
  └─────────────────────────┴───────┴───────┴─┘    valid size: 3..=23
                                                   (4 KB .. 4 GB)
```

Note `virtual_size` is not the physical size. A 32 KB ITCM configured with a
128 KB virtual size _mirrors_ four times. That is why the page-table mapping
(Chapter 5) walks the whole virtual range rather than just the physical extent.

The ranges these produce are exposed to the memory layer as plain Rust ranges
([mem/cp15.rs:81-87](core/src/hw/mem/cp15.rs#L81-L87)):

```rust
pub fn itcm_range(&self) -> Range<u32> {
    0..self.itcm_control.virtual_size
}

pub fn dtcm_range(&self) -> Range<u32> {
    self.dtcm_control.base..self.dtcm_control.base + self.dtcm_control.virtual_size
}
```

ITCM's base is asserted to stay zero, since hardware ignores it
([mem/cp15.rs:184-193](core/src/hw/mem/cp15.rs#L186-L193)).

---

## 4.4 WAIT_FOR_IRQ — the halt that makes emulation fast

Writing 0 to CP15 `C7,C0,4` halts the ARM9 until an interrupt arrives. It is
one line in Lunaris ([mem/cp15.rs:160-163](core/src/hw/mem/cp15.rs#L160-L163)):

```rust
fn write_cache_command(&mut self, m: u32, p: u32, value: u32) {
    match (m, p) {
        (0, 4) if value == 0 => self.arm9_halted = true,
```

and it interacts with three other places:

```text
   game code                    Lunaris
   ─────────                    ───────
   MCR p15,0,r0,c7,c0,4   ──►   cp15.arm9_halted = true
                                        │
   ARM<true>::emulate     ──►   is_halted() → true
                                        │
                                self.cycle = target;  return;   ← no work done
                                        │
   VBlank IRQ fires       ──►   handle_irq(): cp15.arm9_halted = false
                                        │
                                execution resumes at FFFF_0018h
```

See [arm.rs:169-171](core/src/arm.rs#L169-L171) and
[arm.rs:189-193](core/src/arm.rs#L189-L193). The ARM7's equivalent is the
`HALTCNT` I/O register rather than a coprocessor write, which is why
`is_halted` branches on `IS_ARM9`.

---

## 4.5 What is _not_ implemented

This is the least complete subsystem in the emulator relative to hardware, and
it is deliberate — none of the missing pieces are observable to a well-behaved
retail game.

**Caches.** The ARM946E-S has 8 KB I-cache and 4 KB D-cache. Lunaris implements
no cache at all; the commands are accepted and logged
([mem/cp15.rs:160-174](core/src/hw/mem/cp15.rs#L160-L174)):

```rust
        (5, 0) if value == 0 => info!("Invalidate Entire Instruction Cache"), // TODO: Invalidate Entire Instruction Cache
        (5, 1) => info!("Invalidate Instruction Cache Line 0x{:X}", value), // TODO: Invalidate Instruction Cache Line
        (6, 0) if value == 0 => info!("Invalidate Entire Data Cache"), // TODO: Invalidate Entire Data Cache
        // ...
        (10, 4) if value == 0 => info!("Drain Write Buffer"), // TODO: Drain Write Buffer
```

Because every memory access in Lunaris goes to the real backing store
immediately, a missing cache is _safe_ — it can never serve stale data. It only
costs timing accuracy (Chapter 3, §3.8). melonDS models the caches optionally
in `src/CP15.cpp`.

**Protection unit.** The eight PU regions and the access-permission registers
are stored faithfully ([mem/cp15.rs:152-158](core/src/hw/mem/cp15.rs#L152-L159))
but never consulted on a memory access. Consequently:

- no data aborts, no prefetch aborts
- ABT mode is unreachable (Chapter 3, §3.3)
- a game that reads unmapped memory gets a `warn!` and zero, instead of an abort

For retail software this is invisible; homebrew that deliberately probes for
aborts would misbehave.

**Cachability / bufferability bits (C2, C3).** Logged only.

```text
   Fidelity of the ARM9 memory-system model
   ┌──────────────────────┬─────────────────────────────────────────┐
   │ TCM mapping          │ ████████████████████  full              │
   │ Vector base select   │ ████████████████████  full              │
   │ WAIT_FOR_IRQ halt    │ ████████████████████  full              │
   │ Control reg bits     │ ██████████████░░░░░░  stored, partly used│
   │ PU regions           │ ██████░░░░░░░░░░░░░░  stored, not enforced│
   │ I/D caches           │ ░░░░░░░░░░░░░░░░░░░░  absent             │
   │ Memory access timing │ ░░░░░░░░░░░░░░░░░░░░  flat 1 cycle       │
   └──────────────────────┴─────────────────────────────────────────┘
```

---

[← 3. The ARM CPU Cores](03_arm_cpu.md) | [Next: 5. Memory Map and Page Tables →](05_memory_map.md)
