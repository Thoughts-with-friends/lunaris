# 17. RTC, Keypad and the Maths Units

Three small peripherals that are easy to skip and expensive to get wrong. None
of them is complicated; all three have exact behaviours that games depend on.

GBATEK references:
[Real-time clock](https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc) ·
[DS keypad (EXTKEYIN)](https://problemkaputt.de/gbatek.htm#dskeypad) ·
[GBA keypad (KEYINPUT/KEYCNT)](https://problemkaputt.de/gbatek.htm#gbakeypadinput) ·
[DS maths](https://problemkaputt.de/gbatek.htm#dsmaths)

---

## 17.1 The keypad

Twelve buttons split across two registers, because the DS inherited the GBA's
ten and added X and Y.

```text
   KEYINPUT  4000130h  — both CPUs
    9  8  7  6  5  4  3  2  1  0
   ┌──┬──┬──┬──┬──┬──┬──┬──┬──┬──┐
   │ L│ R│Dn│Up│Lf│Rt│St│Se│ B│ A│      ALL BITS ACTIVE-LOW
   └──┴──┴──┴──┴──┴──┴──┴──┴──┴──┘      0 = pressed, 1 = released

   EXTKEYIN  4000136h  — ARM7 only
    7  6  ...  3  2  1  0
   ┌──┬──┬──────┬──┬──┬──┐
   │HG│PN│  -   │DB│ - │ Y│ X│           HG = hinge (lid closed)
   └──┴──┴──────┴──┴──┴──┘               PN = pen down (touchscreen)
                                          DB = debug button
```

Active-low is the single most common source of "the game thinks every button is
held" bugs. Lunaris keeps it explicit
([keypad.rs:45-59](core/src/hw/keypad.rs#L45-L59)):

```rust
    pub fn press_key(&mut self, key: Key) {
        if (key as usize) < 10 {
            self.keyinput.bits &= !(1 << (key as usize));
        } else {
            self.extkeyin.bits &= !(1 << (key as usize - 10));
        }
    }

    pub fn release_key(&mut self, key: Key) {
        if (key as usize) < 10 {
            self.keyinput.bits |= 1 << (key as usize);
        } else {
            self.extkeyin.bits |= 1 << (key as usize - 10);
        }
    }
```

The `Key` enum's discriminants encode which register a button lives in
([keypad.rs:17-31](core/src/hw/keypad.rs#L17-L31)) — X = 10 and Y = 11 fall
into the EXTKEYIN branch by construction.

### The keypad interrupt is a _combination_ test

Unlike every other IRQ source, the keypad one is not an event. It is a
continuously evaluated condition over the current button state
([keypad.rs:74-85](core/src/hw/keypad.rs#L74-L85)):

```rust
    pub fn interrupt_requested(&self) -> bool {
        if self.keycnt.contains(KEYCNT::IRQ_ENABLE) {
            let irq_keys = self.keycnt - KEYCNT::IRQ_ENABLE - KEYCNT::IRQ_COND_AND;
            if self.keycnt.contains(KEYCNT::IRQ_COND_AND) {
                irq_keys.bits() & !self.keyinput.bits() == irq_keys.bits()
            } else {
                irq_keys.bits() & !self.keyinput.bits() != 0
            }
        } else {
            false
        }
    }
```

```text
   KEYCNT  4000132h
    15 14 13        10  9 ...  0
   ┌──┬──┬─────────────┬─────────┐
   │EN│&&│      -      │ mask    │
   └──┴──┴─────────────┴─────────┘
     │  └── 0 = OR  (any selected key pressed)
     │      1 = AND (ALL selected keys pressed simultaneously)
     └───── IRQ enable

   AND mode is what "soft reset" combos use:
     mask = L|R|Start|Select, cond = AND
     → IRQ only when all four are held at once
```

Because it is level-based rather than edge-based, it is polled from the
interrupt check rather than pushed (Chapter 7, §7.1):

```rust
pub fn arm7_interrupts_requested(&mut self) -> bool {
    if unlikely(self.keypad.interrupt_requested()) {
        self.interrupts[0].request |= InterruptRequest::KEYPAD
    }
    // ...
```

---

## 17.2 The real-time clock

The RTC is a Seiko S-35180 **bit-banged over three GPIO lines** in a single
register. There is no hardware serialiser — the ARM7 toggles clock and data by
hand ([rtc.rs:1-11](core/src/hw/rtc.rs#L1-L11)):

```text
   RTC register  4000138h
    7  6  5  4     3  2  1  0
   ┌───────────┬──┬──┬──┬──┬──┐
   │ direction │  │CS│SCK│DATA│
   │   bits    │  │  │   │    │
   └───────────┴──┴──┴──┴──────┘

   The ARM7 writes DATA, toggles SCK 0→1 to clock a bit in, eight times
   per byte, with CS held high for the whole transaction.
```

```rust
pub struct RTC {
    // Register
    data: bool,
    sck: bool,
    cs: bool,
    sck_write: bool,
    data_write: bool,
    cs_write: bool,

    mode: Mode,
    last_byte: bool,
    date_time: DateTime,
}
```

([rtc.rs:22-34](core/src/hw/rtc.rs#L22-L34))

The `*_write` booleans are the GPIO **direction** bits — whether the ARM7 is
driving that line or reading it. Reading a line the CPU is driving must return
what the CPU wrote, not what the chip would say.

### The protocol as a state machine

[rtc.rs:241-258](core/src/hw/rtc.rs#L241-L258):

```rust
enum Mode {
    StartCmd(bool),
    SetCmd(u8, usize),
    ExecCmd(Parameter, AccessType),
    EndCmd,
}

enum Parameter {
    StatusReg1,
    StatusReg2,
    DateTime(u8),
    Time(u8),
    Alarm1FreqDuty(u8),
    Alarm2(u8),
    ClockAdjust,
}
```

```text
   CS rises
      │
      ▼
   StartCmd ──► SetCmd: collect 8 command bits
      │            │
      │            │  command byte:  0110 RRR D
      │            │                 ^^^^ ^^^ ^
      │            │                 │    │   └ 0=write 1=read
      │            │                 │    └──── register select
      │            │                 └───────── fixed code 0110
      ▼            ▼
   ExecCmd(Parameter, AccessType)
      │   clock parameter bytes in or out, BCD-encoded
      ▼
   EndCmd  (CS falls)
```

The fixed `0110` prefix is validated against `RTC::COMMAND_CODE`
([rtc.rs:37](core/src/hw/rtc.rs#L37)).

### Where the time comes from

Lunaris reads the **host clock** via `chrono`:

```rust
use chrono::{Datelike, Timelike, offset::Local};
```

```text
   DATE register (7 bytes, all BCD)
   ┌──────┬───────┬─────┬────────┬──────┬────────┬────────┐
   │ year │ month │ day │ weekday│ hour │ minute │ second │
   └──────┴───────┴─────┴────────┴──────┴────────┴────────┘
     00-99   01-12  01-31   0-6     0-23    0-59     0-59

   BCD: the decimal digits are stored in nibbles.
        43 decimal  →  0x43,  NOT 0x2B
```

Getting BCD wrong shows up as a game displaying an impossible date, or — more
often — the DS system clock reading "1 January 2000" forever.

> **Design note:** sourcing from the host clock means a savestate loaded a day
> later resumes with a day-later RTC. Games that gate events on real time
> (Pokémon berry growth, Animal Crossing) behave as if the console were left
> running. That is arguably the _correct_ behaviour, but it is a choice.

---

## 17.3 The maths accelerators

The ARM9 has no hardware divide instruction, so the DS provides one as a
peripheral — plus a square-root unit.

```text
   DIVCNT   4000280h    mode + busy + div-by-zero flags
   DIV_NUMER 4000290h   64-bit numerator
   DIV_DENOM 4000298h   64-bit denominator
   DIV_RESULT 40002A0h  64-bit quotient
   DIVREM_RESULT 40002A8h 64-bit remainder

   mode 0: 32 / 32 → 32, 32
   mode 1: 64 / 32 → 64, 32
   mode 2: 64 / 64 → 64, 64
   mode 3: reserved — behaves as mode 1
```

Lunaris recomputes on every parameter write rather than modelling a busy period
([math.rs:88-98](core/src/hw/math.rs#L88-L98)):

```rust
    pub fn write_numer(&mut self, scheduler: &mut Scheduler, byte: usize, value: u8) {
        self.numer.write(scheduler, byte, value);
        self.calc();
    }
    pub fn write_denom(&mut self, scheduler: &mut Scheduler, byte: usize, value: u8) {
        self.denom.write(scheduler, byte, value);
        self.calc();
    }
```

That means the result is available immediately — one byte into a 64-bit
parameter write, the unit has already produced a (meaningless) intermediate
answer. It is correct by the time the last byte lands, which is all software
observes.

### The edge cases are the whole point

[math.rs:44-75](core/src/hw/math.rs#L44-L75):

```rust
        self.cnt.div_by_0 = self.denom.value == 0;
        let (numer, denom) = match self.cnt.mode {
            0 => (self.numer.value as u32 as i32 as i64, self.denom.value as u32 as i32 as i64),
            // Although 3 is reserved, it is used with `kingdom hearts 365`, and according to the reference below, it is apparently equivalent to 1.
            // ref: https://problemkaputt.de/gbatek.htm#dsmaths
            1 | 3 => (self.numer.value as i64, self.denom.value as u32 as i32 as i64),
            2 => (self.numer.value as i64, self.denom.value as i64),
            _ => unreachable!(),
        };
        let special_invert = |num: &mut u64| *num ^= 0xFFFF_FFFF_0000_0000;
        if numer == i64::MIN && denom == -1 {
            self.quot.value = numer as u64;
            self.rem.value = 0;
            if self.cnt.mode == 0 {
                special_invert(&mut self.quot.value)
            }
        } else if denom == 0 {
            if numer == 0 {
                self.quot.value = -1i64 as u64;
            } else {
                self.quot.value = (-numer.signum()) as u64;
            }
            self.rem.value = numer as u64;
            if self.cnt.mode == 0 {
                special_invert(&mut self.quot.value)
            }
        } else {
            self.quot.value = (numer / denom) as u64;
            self.rem.value = (numer % denom) as u64;
        }
```

```text
   case                       quotient              remainder
   ────────────────────────   ───────────────────   ──────────
   normal                     numer / denom         numer % denom
   denom == 0, numer == 0     -1                    numer (0)
   denom == 0, numer != 0     -sign(numer)          numer
   i64::MIN / -1 (overflow)   i64::MIN              0

   plus, in mode 0 only: the upper 32 bits of the quotient are INVERTED
                         (special_invert: XOR 0xFFFF_FFFF_0000_0000)
```

That upper-word inversion in mode 0 is genuine documented hardware behaviour,
not a workaround — and a `0 / 0` in a game's maths library would otherwise read
back a plausible-looking wrong number instead of the exact value hardware gives.

The `1 | 3` arm is a nice example of a spec-vs-reality note kept in the code:
mode 3 is "reserved", and Kingdom Hearts 358/2 Days uses it anyway.

### Square root

```rust
    pub fn write_param(&mut self, scheduler: &mut Scheduler, byte: usize, value: u8) {
        self.param.write(scheduler, byte, value);
        // TODO: Take correct num of cycles
        self.result = if self.cnt.is_64bit {
            self.param.value.sqrt() as u32
        } else {
            (self.param.value as u32).sqrt()
        };
    }
```

([math.rs:123-131](core/src/hw/math.rs#L123-L131))

Integer square root, truncating, via `num_integer::Roots`. SQRTCNT bit 0
selects 32- or 64-bit input; the result is always 32-bit.

---

## 17.4 Divergences

- **No busy timing on either maths unit.** Both are marked
  `// TODO: Take correct num of cycles`. Real hardware takes 18 cycles (div) or
  13 (sqrt) and reports busy meanwhile. A game that polls the busy flag exits
  its loop on the first read, which is harmless; a game that _relies_ on the
  delay for timing would run slightly fast.
- **RTC alarms are decoded but not fired.** `Alarm1FreqDuty` and `Alarm2`
  parameters are handled in the protocol, but nothing raises an interrupt from
  them.
- **The RTC cannot be set.** Writes to the date/time registers are accepted by
  the protocol but the host clock remains the source of truth.
- **The hinge switch (EXTKEYIN bit 7) is never asserted** — there is no lid to
  close, so games' sleep-on-close paths are never exercised.
- **The debug button (EXTKEYIN bit 3)** is likewise always released.

---

[← 16. SPI: Firmware, Touchscreen, Power](16_spi_firmware_touchscreen.md) | [Next: 18. Wi-Fi and Local Multiplayer →](18_wifi_and_local_mp.md)
