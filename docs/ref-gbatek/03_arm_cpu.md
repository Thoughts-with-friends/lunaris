# 3. The ARM CPU Cores

The DS has two ARM cores. Lunaris implements both with **one** interpreter,
generic over a const boolean.

GBATEK references:
[CPU overview](https://problemkaputt.de/gbatek.htm#armcpuoverview) ·
[Register set](https://problemkaputt.de/gbatek.htm#armcpuregisterset) ·
[Exceptions](https://problemkaputt.de/gbatek.htm#armcpuexceptions) ·
[Instruction cycle times](https://problemkaputt.de/gbatek.htm#armcpuinstructioncycletimes)

---

## 3.1 Two cores, one implementation

```text
                    ARM<IS_ARM9>          core/src/arm.rs
                          │
          ┌───────────────┴────────────────┐
          │                                │
   ARM<true>  = ARM946E-S           ARM<false> = ARM7TDMI
   ├ ARMv5TE                        ├ ARMv4T
   ├ 67.03 MHz (2× master)          ├ 33.51 MHz (1× master)
   ├ CLZ, QADD/QSUB, BLX,           ├ no CLZ, no BLX, no saturating
   │ signed halfword MUL            │ arithmetic
   ├ CP15: TCM + protection unit    ├ no coprocessor
   ├ vector base relocatable        ├ vectors fixed at 0000_0000h
   │ (0000_0000h or FFFF_0000h)     │
   └ owns main RAM, VRAM, 3D        └ owns audio, Wi-Fi, SPI, RTC
```

Everything that differs is behind `if IS_ARM9`, resolved at compile time, so
neither core pays for the other's features.

---

## 3.2 ARM7TDMI overview

- **32-bit RISC CPU**
- Designed for **high performance** and **low power consumption**

### Pipeline

```text
   ┌────────┐   ┌────────┐   ┌─────────┐
   │ Fetch  │──►│ Decode │──►│ Execute │
   └────────┘   └────────┘   └─────────┘
       ▲
       │  PC always points *two instructions ahead* of the one executing:
       │    ARM state   → PC = executing_addr + 8
       │    THUMB state → PC = executing_addr + 4
```

Three-stage pipeline enables simultaneous instruction fetch, decode, and
execution. The "PC reads ahead" rule is not cosmetic: it is observable to
software, and getting it wrong breaks every PC-relative load.

Lunaris models the pipeline with a two-word prefetch buffer rather than
simulating three stages ([arm.rs:56-57](core/src/arm.rs#L56-L57)):

```rust
    /// Two-word prefetch pipeline buffer (`[0]` = current, `[1]` = next).
    instr_buffer: [u32; 2],
```

and refills it on every branch ([arm/arm.rs:5-11](core/src/arm/arm.rs#L5-L11)):

```rust
pub(super) fn fill_arm_instr_buffer(&mut self, hw: &mut HW) {
    self.regs[15] &= !0x3;
    self.instr_buffer[0] = self.read::<u32>(hw, AccessType::S, self.regs[15] & !0x3);
    self.regs[15] = self.regs[15].wrapping_add(4);

    self.instr_buffer[1] = self.read::<u32>(hw, AccessType::S, self.regs[15] & !0x3);
}
```

```text
   instr_buffer[0]  instr_buffer[1]         R15 (PC)
        │                │                     │
   ┌────▼────┐      ┌────▼────┐           ┌────▼────┐
   │executing│      │ decoded │           │ next to │
   │  now    │      │  next   │           │  fetch  │
   └─────────┘      └─────────┘           └─────────┘
     addr+0            addr+4                addr+8
```

### Data Types

| Size   | Type     |
| ------ | -------- |
| 8-bit  | Byte     |
| 16-bit | Halfword |
| 32-bit | Word     |

### CPU States

| ARM State                           | THUMB State                         |
| ----------------------------------- | ----------------------------------- |
| 32-bit instructions                 | 16-bit instructions                 |
| Higher performance on 32-bit memory | Better performance on 16-bit memory |
| Access to R0–R15                    | Most instructions use R0–R7         |
| Larger code size                    | Smaller code size                   |

Both states use the same **32-bit registers** and **32-bit address space**.

### State Switching

```text
   ARM ⇄ THUMB   (BX instruction; low bit of the target address selects)

   BX Rn   where Rn = 0x02000101   →  THUMB, jump to 0x02000100
   BX Rn   where Rn = 0x02000100   →  ARM,   jump to 0x02000100
```

Lunaris implements exactly that ([arm/arm.rs:63-70](core/src/arm/arm.rs#L63-L70)):

```rust
    // BX
    self.regs[15] = self.regs[instr & 0xF];
    if self.regs[15] & 0x1 != 0 {
        self.regs[15] -= 1;
        self.regs.set_t(true);
        self.fill_thumb_instr_buffer(hw);
    } else {
        self.fill_arm_instr_buffer(hw)
    }
```

### Exceptions

```text
Exception → ARM State
Return    → Previous State
```

Exceptions automatically enter **ARM state** and restore the previous state on
return.

---

## 3.3 The register file and banked modes

```text
   Mode:     USR/SYS      FIQ        IRQ        SVC        ABT        UND
             ───────    ───────    ───────    ───────    ───────    ───────
   R0-R7  │  shared across every mode
   R8     │   R8      │  R8_fiq  │   R8     │   R8     │   R8     │   R8
   R9     │   R9      │  R9_fiq  │    …
   R10-12 │           │ R10-12_fiq
   R13 SP │  R13      │ R13_fiq  │ R13_irq  │ R13_svc  │ R13_abt  │ R13_und
   R14 LR │  R14      │ R14_fiq  │ R14_irq  │ R14_svc  │ R14_abt  │ R14_und
   R15 PC │  shared
   CPSR   │  shared
   SPSR   │   —       │ SPSR_fiq │ SPSR_irq │ SPSR_svc │ SPSR_abt │ SPSR_und
```

Lunaris keeps the _visible_ sixteen registers in one array and swaps banked
copies in and out on a mode change
([arm/registers.rs:76-90](core/src/arm/registers.rs#L76-L90)):

```rust
pub struct RegValues {
    regs: [u32; 16],
    svc: [u32; 2], // R13 and R14
    und: [u32; 2], // R13 and R14
    irq: [u32; 2], // R13 and R14
    fiq: [u32; 7], // R8-R14
    cpsr: StatusReg,
```

`change_mode` performs the swap in the only order that is safe — save the
outgoing bank first, then switch, then load
([arm/registers.rs:132-139](core/src/arm/registers.rs#L132-L139)):

```rust
pub fn change_mode(&mut self, mode: Mode) {
    self.save_banked();
    let cpsr = self.cpsr();
    self.cpsr.set_mode(mode);
    self.load_banked(mode);
    *self.spsr_mut() = cpsr;
    self.update_spsr_mode();
}
```

`StatusReg` caches the decoded `Mode` alongside the raw bits so the hot path
never re-matches on bits [4:0].

> **Divergence:** ABT (abort) mode is present in the enum but
> [`save_banked`](core/src/arm/registers.rs#L154-L162) treats it as
> `unreachable!()`. Lunaris does not emulate data/prefetch aborts, because the
> CP15 protection unit is not enforced (Chapter 4).

---

## 3.4 Instruction dispatch: precomputed lookup tables

Decoding an ARM instruction with a chain of `if (instr & mask) == pattern` on
every single execution is the slowest possible design. Lunaris instead builds
the decode table **once per process**, at startup:

```text
   ARM decode index (4096 entries)
   ┌────────────────────────────────────────────────────────────┐
   │ 31   28│27           20│19        8│7    4│3     0│        │
   │  cond  │  ▓▓▓▓▓▓▓▓     │           │ ▓▓▓▓ │       │        │
   └────────┴───────────────┴───────────┴──────┴───────┴────────┘
                  │                        │
                  └────────┬───────────────┘
                           ▼
            index = (instr >> 16 & 0xFF0) | (instr >> 4 & 0xF)
                    ^^^^^ bits 27..20 ^^^^   ^^ bits 7..4 ^^
                                 = 12 bits = 4096 entries

   THUMB decode index (256 entries)
   ┌──────────────┬──────────────┐
   │15          8 │7           0 │
   │  ▓▓▓▓▓▓▓▓    │              │      index = instr >> 8
   └──────────────┴──────────────┘             = 8 bits = 256 entries
```

The tables are `OnceLock` statics ([arm.rs:32-35](core/src/arm.rs#L32-L35),
[arm.rs:429-445](core/src/arm.rs#L429-L445)) built by
[`arm::gen_lut`](core/src/arm/arm.rs#L853-L861):

```rust
pub(super) fn gen_lut<const IS_ARM9: bool>() -> [InstructionHandler<u32, IS_ARM9>; 4096] {
    // Bits 0-3 of opcode = Bits 4-7 of instr
    // Bits 4-11 of opcode = Bits Bits 20-27 of instr
    let mut lut: [InstructionHandler<u32, IS_ARM9>; 4096] = [ARM::undefined_instr_arm; 4096];
```

The clever part is `compose_instr_handler!`
([arm/instructions.rs:5-19](core/src/arm/instructions.rs#L5-L19)). Instruction
flag bits (pre/post index, up/down, byte/word, write-back, load/store …) are
_known_ once the table index is known — so instead of testing them at runtime,
the macro recursively expands the handler into a **const-generic
monomorphisation** with those flags baked in:

```rust
macro_rules! compose_instr_handler {
    ($handler: ident, $skeleton: expr, $($bit: expr),* ) => {
        compose_instr_handler!($handler, flags => (), values => ( $($skeleton >> $bit & 0x1 != 0),* ))
    };
    ($handler: ident, flags => ( $( $flag: expr),* ), values => ()) => {
        ARM::$handler::<$($flag,)*>
    };
    ($handler: ident, flags => ( $($flag: expr),* ), values => ( $cur_value:expr $( , $value: expr )* )) => {
        if $cur_value {
            compose_instr_handler!($handler, flags => ( $($flag,)* true ), values => ( $($value),* ))
        } else {
            compose_instr_handler!($handler, flags => ( $($flag,)* false ), values => ( $($value),* ))
        }
    };
}
```

So `compose_instr_handler!(data_proc, skeleton, 25, 24, 23, 22, 21, 20, 6, 5, 4)`
expands to 512 distinct specialised copies of `data_proc`, and the table entry
points at exactly the right one. Zero flag tests remain in the hot loop.

---

## 3.5 The execution step

[arm/arm.rs:13-48](core/src/arm/arm.rs#L13-L48), trace logging elided:

```rust
pub(super) fn emulate_arm_instr(&mut self, hw: &mut HW) {
    let instr = self.instr_buffer[0];
    // ... trace! ...
    self.instr_buffer[0] = self.instr_buffer[1];
    self.regs[15] = self.regs[15].wrapping_add(4);

    if likely(self.should_exec((instr >> 28) & 0xF)) {
        self.arm_lut[((instr as usize) >> 16 & 0xFF0) | ((instr as usize) >> 4 & 0xF)](
            self, hw, instr,
        );
    } else {
        self.instruction_prefetch::<u32>(hw, AccessType::S);
    }
}
```

Note the failed-condition path still performs a prefetch — a skipped
instruction is not free on real hardware, it still costs its fetch cycle.

### Condition codes

Every ARM instruction is predicated on bits [31:28]. Lunaris evaluates that
with a 256-entry table indexed by `(NZCV << 4) | cond`, generated `const`
([arm/instructions.rs:21-58](core/src/arm/instructions.rs#L21-L58)):

```rust
pub(super) const fn gen_condition_table() -> [bool; 256] {
    // ...
            lut[flags << 4 | condition] = match condition {
                0x0 => z,       // EQ
                0x1 => !z,      // NE
                0x2 => c,       // CS
                0x3 => !c,      // CC
                // ...
                0xE => true,    // AL
                0xF => true, // True so that some ARMv5 instructions can execute
```

Condition `0xF` is "never" on ARMv4 but is reused on ARMv5 as the encoding
space for BLX and PLD — hence returning `true`.

---

## 3.6 The main run loop and IRQ entry

[arm.rs:110-124](core/src/arm.rs#L110-L124):

```rust
pub fn emulate(&mut self, hw: &mut HW, target: usize) {
    while self.cycle < target {
        self.handle_irq(hw);
        if self.is_halted(hw) {
            self.cycle = target;
            return;
        }

        if unlikely(self.regs.get_t()) {
            self.emulate_thumb_instr(hw)
        } else {
            self.emulate_arm_instr(hw)
        }
    }
}
```

A halted CPU (`WAIT_FOR_IRQ` on ARM9 via CP15, `HALTCNT` on ARM7) does not
spin — it fast-forwards its counter straight to the target. That single line
is what makes the emulator fast during the ~60% of frame time a typical game
spends waiting in HALT.

IRQ entry follows GBATEK's exception sequence exactly
([arm.rs:179-206](core/src/arm.rs#L179-L206)):

```text
   IRQ asserted
        │
        ├─ 1. un-halt the CPU
        ├─ 2. change_mode(IRQ)          → banks R13/R14, saves CPSR to SPSR_irq
        ├─ 3. LR = PC of interrupted instruction + 4
        ├─ 4. clear T (force ARM state)
        ├─ 5. set I (mask further IRQs)
        ├─ 6. PC = interrupt_base | 0x18
        │        ARM7: base = 0000_0000h  → 0000_0018h
        │        ARM9: base = FFFF_0000h  → FFFF_0018h  (CP15 selectable)
        └─ 7. refill the ARM prefetch buffer
```

```rust
    self.regs.change_mode(Mode::IRQ);
    let lr = if unlikely(self.regs.get_t()) {
        self.read::<u16>(hw, AccessType::N, self.regs[15]);
        self.regs[15].wrapping_sub(2).wrapping_add(4)
    } else {
        self.read::<u32>(hw, AccessType::N, self.regs[15]);
        self.regs[15].wrapping_sub(4).wrapping_add(4)
    };
    self.regs.set_lr(lr);
    self.regs.set_t(false);
    self.regs.set_i(true);
    self.regs[15] = interrupt_base | 0x18;
    self.fill_arm_instr_buffer(hw);
```

The `self.read::<…>()` whose value is thrown away is deliberate: it charges the
cycle cost of the aborted prefetch.

---

## 3.7 ALU details worth copying

**SUB implemented as ADC.** Rather than writing separate borrow logic, Lunaris
routes subtraction through the adder, which is how the silicon does it
([arm.rs:369-380](core/src/arm.rs#L369-L380)):

```rust
/// SUB: `op1 - op2` implemented as `op1 + NOT(op2) + 1` via [`adc`](Self::adc).
pub(self) fn sub(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
    let old_c = self.regs.get_c();
    self.regs.set_c(true);
    let result = self.adc(op1, !op2, change_status); // Simulate adding op1 + !op2 + 1
    if !change_status {
        self.regs.set_c(old_c)
    }
    result
}
```

This gets the carry/overflow flags right for free, which hand-written `sub`
flag logic almost never does on the first attempt.

**The barrel shifter's special cases.** Shift amount 0 encoded as an immediate
does _not_ mean "shift by zero" for LSR/ASR/ROR
([arm.rs:228-258](core/src/arm.rs#L228-L258)):

```text
   encoded shift == 0, immediate form:
     LSL #0  →  operand unchanged, C unchanged
     LSR #0  →  interpreted as LSR #32   → result 0, C = operand[31]
     ASR #0  →  interpreted as ASR #32   → result 0 or ~0, C = operand[31]
     ROR #0  →  interpreted as RRX #1    → C rotated in at bit 31
```

**Multiply timing.** The ARM multiplier early-outs on leading zero (or, for
signed multiplies, leading sign) bytes; Lunaris charges cycles the same way
([arm.rs:386-399](core/src/arm.rs#L386-L399)):

```rust
pub(self) fn inc_mul_clocks(&mut self, op1: u32, signed: bool) {
    let mut mask = 0xFF_FF_FF_00;
    loop {
        self.internal();
        let value = op1 & mask;
        if mask == 0 || value == 0 || signed && value == mask {
            break;
        }
        mask <<= 8;
    }
}
```

---

## 3.8 Memory access cycles

Every CPU memory access charges time and records whether the _next_ access will
be sequential ([arm.rs:126-155](core/src/arm.rs#L126-L155)):

```rust
pub fn read<T: MemoryValue>(&mut self, hw: &mut HW, access_type: AccessType, addr: u32) -> T {
    let value = if IS_ARM9 {
        let value = hw.arm9_read::<T>(addr);
        self.cycle += hw.arm9_get_access_time::<T>(self.next_access_type, addr);
        value
    } else {
        let value = hw.arm7_read::<T>(addr);
        self.cycle += hw.arm7_get_access_time::<T>(self.next_access_type, addr);
        value
    };
    self.next_access_type = access_type;
    value
}
```

```text
   N-cycle (Non-sequential)  first access to a new address
   S-cycle (Sequential)      access to address+4 following an N or S
   I-cycle (Internal)        no bus activity (shifts, multiplies)
```

> **Divergence — flat memory timings.** `arm9_get_access_time` and
> `arm7_get_access_time` currently return a constant 1 regardless of region or
> access type ([mem/arm9.rs:132-139](core/src/hw/mem/arm9.rs#L132-L139)):
>
> ```rust
> pub fn arm9_get_access_time<T: MemoryValue>(
>     &mut self,
>     _access_type: AccessType,
>     _addr: u32,
> ) -> usize {
>     // TODO: Use accurate timings
>     1
> }
> ```
>
> Real hardware charges very different costs for main RAM, TCM, VRAM and the
> cartridge bus (see
> [DS Memory Timings](https://problemkaputt.de/gbatek.htm#dsmemorytimings)).
> Games that are timing-sensitive in ways VBlank does not paper over may run
> subtly fast. melonDS implements the full table in `src/NDS.cpp` /
> `src/ARM.cpp`; this is the largest known accuracy gap in the Lunaris CPU
> layer.

Also not emulated: the ARM9 instruction and data **caches**. CP15 cache
commands are accepted and logged but do nothing (Chapter 4).

---

## 3.9 Reset state under direct boot

Direct boot skips the firmware, so the registers must be seeded with what the
firmware would have left ([arm/registers.rs:109-130](core/src/arm/registers.rs#L109-L130)):

```rust
pub fn direct_boot<const IS_ARM9: bool>(pc: u32) -> RegValues {
    let mut reg_values = RegValues::new::<IS_ARM9>();
    // ...
    reg_values.cpsr.bits.0 = 0xD3;          // SVC mode, IRQ+FIQ disabled, ARM state
    reg_values.cpsr.update_mode();
    if IS_ARM9 {
        reg_values.svc[0] = 0x03003FC0;     // SVC stack
        reg_values.irq[0] = 0x03003F80;     // IRQ stack
        reg_values.regs[13] = 0x03002F7C;   // user stack
    } else {
        reg_values.svc[0] = 0x0380FFC0;
        reg_values.irq[0] = 0x0380FF80;
        reg_values.regs[13] = 0x0380FD80;
    };
    assert_eq!(reg_values.get_mode(), Mode::SVC);
    reg_values.regs[15] = pc;
    reg_values
}
```

```text
   CPSR = 0xD3
   ┌───┬───┬───┬─────────┐
   │ I │ F │ T │  Mode   │
   │ 1 │ 1 │ 0 │ 1 0011  │  = SVC, both interrupt lines masked, ARM state
   └───┴───┴───┴─────────┘
```

---

[← 2. Workspace and Code Layout](02_workspace_layout.md) | [Next: 4. CP15, TCM and the Protection Unit →](04_cp15_and_tcm.md)
