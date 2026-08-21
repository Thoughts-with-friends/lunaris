# 5. Memory Map and Page Tables

Two CPUs, two different views of the same silicon. This chapter covers how
Lunaris decodes an address, and the raw-pointer trick that keeps it fast.

GBATEK references:
[DS memory maps](https://problemkaputt.de/gbatek.htm#dsmemorymaps) ·
[I/O maps](https://problemkaputt.de/gbatek.htm#dsiomaps) ·
[WRAM control](https://problemkaputt.de/gbatek.htm#dsmemorycontrolwram) ·
[Memory timings](https://problemkaputt.de/gbatek.htm#dsmemorytimings)

---

## 5.1 The two memory maps side by side

```text
              ARM9 (ARM946E-S)                     ARM7 (ARM7TDMI)
   ┌──────────────────────────────────┐  ┌──────────────────────────────────┐
0000_0000 ITCM (mirrored, CP15)       │  │ ARM7 BIOS            16 KB       │
0000_8000 └─────────────────────────  │  │                                  │
          │                           │  │                                  │
0200_0000 │ Main RAM   4 MB ══════════╪══╪═ Main RAM   4 MB (SAME memory)   │
0240_0000 │  (mirrored to 03000000)   │  │  (mirrored)                      │
          │                           │  │                                  │
0300_0000 │ Shared WRAM ══════════════╪══╪═ Shared WRAM  (WRAMCNT-banked)   │
0380_0000 │      (not visible)        │  │ ARM7 IWRAM  64 KB                │
          │                           │  │                                  │
0400_0000 │ ARM9 I/O registers        │  │ ARM7 I/O registers               │
          │  DISPCNT, DMA, 3D, …      │  │  SPI, SPU, Wi-Fi, RTC, …         │
          │                           │  │                                  │
0500_0000 │ Palette RAM  2 KB         │  │  ─                               │
0600_0000 │ VRAM (bank-mapped)        │  │ VRAM banks C/D if allocated      │
0700_0000 │ OAM  2 KB                 │  │  ─                               │
0800_0000 │ GBA slot ROM              │  │ GBA slot ROM                     │
0A00_0000 │ GBA slot RAM              │  │ GBA slot RAM                     │
          │                           │  │                                  │
FFFF_0000 │ ARM9 BIOS  4 KB           │  │  ─                               │
   └──────────────────────────────────┘  └──────────────────────────────────┘
```

The same tables appear as doc comments at the top of
[hw/mem.rs:9-28](core/src/hw/mem.rs#L9-L28), so the map is visible where the
dispatch code is.

Backing store sizes ([hw.rs:185-189](core/src/hw.rs#L185-L189)):

```rust
const ITCM_SIZE: usize = 0x8000; // 32 KiB
const DTCM_SIZE: usize = 0x4000; // 16 KiB
const MAIN_MEM_SIZE: usize = 0x40_0000; // 4 MiB
const IWRAM_SIZE: usize = 0x1_0000; // 64 KiB
const SHARED_WRAM_SIZE: usize = 0x8000; // 32 KiB
```

---

## 5.2 The core problem: decoding is expensive

A naïve emulator writes:

```rust
match addr >> 24 {
    0x02 => main_mem[(addr & 0x3F_FFFF) as usize],
    0x03 => /* which half of shared WRAM? which CPU? */,
    0x04 => io_read(addr),
    // ...
}
```

That `match` runs on _every_ memory access — several per instruction, tens of
millions per second. Lunaris moves the common case out of the match entirely.

---

## 5.3 Raw-pointer page tables

The address space is cut into fixed-size pages; a flat array maps each page to
a raw pointer into the backing `Vec<u8>`. A null pointer means "not a plain
memory page — take the slow path".

```text
   ARM9: 4 KiB pages (shift 12)      ARM7: 16 KiB pages (shift 14)

   addr = 0x0201_2345
             │
             ├── addr >> 12  = 0x02012  ──► page table index
             └── addr & 0xFFF = 0x345   ──► offset within page

   arm9_page_table
   ┌────────────┬──────────────────────────────────────┐
   │ index      │ *mut u8                              │
   ├────────────┼──────────────────────────────────────┤
   │ 0x00000    │ ──► itcm[0x0000]      (ITCM, if on)  │
   │    ...     │                                      │
   │ 0x02000    │ ──► main_mem[0x000000]               │
   │ 0x02001    │ ──► main_mem[0x001000]               │
   │ 0x02012    │ ──► main_mem[0x012000]  ◀── our page │
   │    ...     │                                      │
   │ 0x03000    │ NULL   (shared WRAM: needs WRAMCNT)  │
   │ 0x04000    │ NULL   (I/O: needs dispatch)         │
   │ 0x06000    │ NULL   (VRAM: needs bank lookup)     │
   │    ...     │                                      │
   │ 0xFFFF0    │ ──► bios9[0x0000]                    │
   └────────────┴──────────────────────────────────────┘
```

The hot path is then a null check and a pointer add
([mem/arm9.rs:19-30](core/src/hw/mem/arm9.rs#L19-L30)):

```rust
pub fn arm9_read<T: MemoryValue>(&mut self, addr: u32) -> T {
    let page_table_ptr = self.arm9_page_table[addr as usize >> HW::ARM9_PAGE_SHIFT];
    if !page_table_ptr.is_null() {
        unsafe {
            let slice = std::slice::from_raw_parts(page_table_ptr, HW::ARM9_PAGE_SIZE);
            HW::read_mem(slice, addr & HW::ARM9_PAGE_TABLE_MASK)
        }
    } else {
        match MemoryRegion::from_addr(addr) {
            // ... slow path
```

`read_mem` itself is an unaligned-tolerant reinterpret
([mem.rs:106-116](core/src/hw/mem.rs#L106-L116)):

```rust
pub fn read_mem<T: MemoryValue>(mem: &[u8], addr: u32) -> T {
    unsafe { *(&mem[addr as usize] as *const u8 as *const T) }
}
```

### Mirroring for free

Building the table is where mirroring is handled — once, at init, rather than
by masking on every access ([hw.rs:559-574](core/src/hw.rs#L559-L574)):

```rust
fn map_page_table(
    page_table: &mut [*mut u8],
    page_shift: usize,
    page_size: usize,
    addr_start: usize,
    addr_end: usize,
    mem: &mut [u8],
) {
    let mem_mask = mem.len() - 1;
    for (page_table_i, addr) in
        (addr_start >> page_shift..).zip((addr_start..addr_end).step_by(page_size))
    {
        let mem_addr = addr & mem_mask;
        page_table[page_table_i] = mem[mem_addr..mem_addr + page_size].as_mut_ptr();
    }
}
```

`addr & mem_mask` wraps automatically, so mapping the 4 MB main RAM across the
16 MB region `0200_0000..0300_0000` produces four identical mirrors:

```text
   0200_0000 ─┐
   0240_0000 ─┼─ all four ranges point at the same 4 MB buffer
   0280_0000 ─┤
   02C0_0000 ─┘
```

This requires every backing buffer to be a power-of-two size — which they all
are.

### Mapping order encodes priority

[mem/arm9.rs:141-178](core/src/hw/mem/arm9.rs#L141-L178) maps main RAM and BIOS
first, then DTCM over the top, then ITCM last:

```rust
    // DTCM has second priority
    let dtcm_range = self.cp15.dtcm_range();
    Self::map_page_table( /* ... */ dtcm_range.start as usize, dtcm_range.end as usize, &mut self.dtcm);
    // ITCM has highest priority
    let itcm_range = self.cp15.itcm_range();
    Self::map_page_table( /* ... */ itcm_range.start as usize, itcm_range.end as usize, &mut self.itcm);
```

Later writes win, so "last mapped = highest priority". Any CP15 TCM
reconfiguration must therefore rebuild the whole ARM9 table, not patch it.

### Why the two CPUs use different page sizes

```text
   ARM9: shift 12 → 4 KiB pages → 1,048,577 entries × 8 bytes ≈ 8 MB table
   ARM7: shift 14 → 16 KiB pages →  262,145 entries × 8 bytes ≈ 2 MB table
```

The ARM9 needs 4 KiB granularity because CP15 TCM bases are 4 KiB-aligned
(`base = value & !0xFFF`, Chapter 4). The ARM7 has no TCM, so it can use
coarser pages and a quarter of the memory.

---

## 5.4 The slow path

When the pointer is null, the address is decoded properly. Each CPU has its own
region enum ([mem/arm9.rs:231-247](core/src/hw/mem/arm9.rs#L231-L247)):

```rust
impl ARM9MemoryRegion {
    pub fn from_addr(addr: u32) -> Self {
        use ARM9MemoryRegion::*;
        match addr >> 24 {
            0x3 => SharedWRAM,
            0x4 => IO,
            0x5 => Palette,
            0x6 => VRAM,
            0x7 => OAM,
            0x8 | 0x9 => GBAROM,
            0xA => GBARAM,
            _ => {
                warn!("Uknown Memory Access: {:X}", addr);
                Unknown
            }
        }
    }
}
```

```text
   memory access
        │
        ├─ page_table[addr >> shift] non-null? ──► direct pointer read/write
        │                                          (main RAM, IWRAM, BIOS, TCM)
        └─ null ──► from_addr(addr)
                        ├─ SharedWRAM ─► WRAMCNT offset/mask (§5.5)
                        ├─ IO         ─► arm9_read_io / arm7_read_io  (§5.6)
                        ├─ Palette    ─► engine A if addr&0x7FFF < 0x400 else B
                        ├─ VRAM       ─► vram.arm9_read  (bank lookup, Ch. 12)
                        ├─ OAM        ─► engine A / B OAM
                        ├─ GBAROM     ─► open-bus pattern
                        └─ Unknown    ─► warn! and return 0
```

### The 8-bit write rule

One of those slow-path arms is a real hardware quirk with a very visible
symptom ([mem/arm9.rs:86-93](core/src/hw/mem/arm9.rs#L86-L93)):

```rust
    // The NDS9 discards 8-bit writes to palette RAM, VRAM and OAM;
    // only 16- and 32-bit writes reach those memories. Letting byte
    // writes through corrupts half of a BGR555 entry, which for
    // palette index 0 renders as a black backdrop.
    //
    // GBATEK "DS Memory Maps" / "LCD VRAM Overview".
    MemoryRegion::Palette | MemoryRegion::VRAM | MemoryRegion::OAM
        if size_of::<T>() == 1 => {}
```

---

## 5.5 WRAMCNT: 32 KB shared between two CPUs

The 32 KB shared WRAM at `0300_0000` can be split between the cores four ways.
Lunaris precomputes an offset and a mask per CPU whenever the register changes
([mem.rs:254-298](core/src/hw/mem.rs#L254-L298)):

```rust
pub struct WRAMCNT {
    value: u8,

    arm7_offset: u32,
    arm7_mask: u32,
    arm9_offset: u32,
    arm9_mask: u32,
}
```

```text
   WRAMCNT   ARM9 gets                ARM7 gets
   ───────   ──────────────────────   ──────────────────────
     0       all 32 KB                nothing (falls back to IWRAM mirror)
             ┌──────────────────┐     ┌──────────────────┐
             │███████████████████│     │                  │
             └──────────────────┘     └──────────────────┘
     1       upper 16 KB              lower 16 KB
             ┌────────┬─────────┐     ┌────────┬─────────┐
             │        │█████████│     │████████│         │
             └────────┴─────────┘     └────────┴─────────┘
     2       lower 16 KB              upper 16 KB
             ┌────────┬─────────┐     ┌────────┬─────────┐
             │████████│         │     │        │█████████│
             └────────┴─────────┘     └────────┴─────────┘
     3       nothing                  all 32 KB
             ┌──────────────────┐     ┌──────────────────┐
             │                  │     │███████████████████│
             └──────────────────┘     └──────────────────┘
```

An unmapped side is not an error on the ARM7 — hardware falls through to an
IWRAM mirror, and Lunaris does the same
([mem/arm7.rs:25-28](core/src/hw/mem/arm7.rs#L25-L28)):

```rust
    MemoryRegion::SharedWRAM if self.wramcnt.arm7_mask == 0 => {
        warn!("Reading from Unmapped ARM7 Shared WRAM: 0x{:X}", addr);
        HW::read_mem(&self.iwram, addr & HW::IWRAM_MASK)
    }
```

Because the split is dynamic, shared WRAM pages are **never** placed in the
page tables — they always take the slow path. That is the deliberate trade:
correctness over speed for the one region whose mapping changes at runtime.

---

## 5.6 I/O dispatch

I/O is decoded by width, then by address
([mem/arm7.rs:76-92](core/src/hw/mem/arm7.rs#L76-L92)):

```rust
fn arm7_read_io<T: MemoryValue>(&mut self, addr: u32) -> T {
    match size_of::<T>() {
        1 => num::cast::<u8, T>(self.arm7_read_io8(addr)).unwrap(),
        2 => num::cast::<u16, T>(self.arm7_read_io16(addr)).unwrap(),
        4 => num::cast::<u32, T>(self.arm7_read_io32(addr)).unwrap(),
        _ => unreachable!(),
    }
}
```

The per-width tables live in [mem/arm7/io.rs](core/src/hw/mem/arm7/io.rs) and
[mem/arm9/io.rs](core/src/hw/mem/arm9/io.rs). Registers themselves implement a
byte-granular trait, so a 32-bit write to a 16-bit register pair does the right
thing ([mem.rs:174-177](core/src/hw/mem.rs#L174-L177)):

```rust
pub trait IORegister {
    fn read(&self, byte: usize) -> u8;
    fn write(&mut self, scheduler: &mut Scheduler, byte: usize, value: u8);
}
```

Note `write` receives the `Scheduler`: writing a register frequently needs to
schedule or cancel an event (start a DMA, restart a timer, kick the SPU). That
one parameter is what lets register writes have _timed_ side effects without a
global back-reference.

---

## 5.7 GBA slot: open bus

No Slot-2 cartridge is emulated. Reads return the pattern real hardware
produces on a floating bus ([mem.rs:85-104](core/src/hw/mem.rs#L85-L104)):

```rust
fn read_gba_rom<T: MemoryValue>(&self, is_arm9: bool, addr: u32) -> T {
    if self.exmem.gba_arm7_access != is_arm9 {
        let cnt = &self.exmem.gba[is_arm9 as usize];
        let value = match cnt.rom_n_access_time {
            0 => (addr / 2) | 0xFE08,
            1 | 2 => addr / 2,
            3 => 0xFFFF,
            _ => unreachable!(),
        } & 0xFFFF;
```

Games check this pattern to detect Slot-2 accessories (rumble paks, the
Guitar Grip, expansion RAM); returning zeros instead would make some of them
misdetect hardware.

---

## 5.8 Savestate interaction

Raw pointers cannot be serialised, and would be garbage on reload anyway.
Both page tables are `#[savestate(skip)]` and rebuilt in `post_load_hw`
(Chapter 19). This is the single most important invariant in the memory layer:

```text
   anything that changes a mapping  ⇒  rebuild the page table
   ────────────────────────────────────────────────────────────
   CP15 TCM base/size write         ⇒  init_arm9_page_tables()
   savestate load                   ⇒  init_arm9 + init_arm7 page tables
   (WRAMCNT and VRAM bank changes need no rebuild — those regions are
    never in the table to begin with)
```

---

## 5.9 Divergences

- **Access timings are flat.** `arm7_get_access_time` /
  `arm9_get_access_time` return 1 unconditionally
  ([mem/arm7.rs:94-104](core/src/hw/mem/arm7.rs#L94-L104)); the comment there
  points at the melonDS source that implements the real table.
- **No alignment faults.** Misaligned accesses reinterpret in place rather than
  rotating (ARM7) or faulting.
- **No bus conflict modelling.** Both CPUs may "access" main RAM in the same
  cycle without contention penalty.
- **No Slot-2 devices** (see §5.7).

---

[← 4. CP15, TCM and the Protection Unit](04_cp15_and_tcm.md) | [Next: 6. The Scheduler and Timers →](06_scheduler_and_timers.md)
