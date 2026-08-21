# 7. Interrupts and Inter-Processor Communication

Two independent interrupt controllers, and two ways for the CPUs to talk to
each other. Together these are what turn "two CPUs running in a loop" into a
working console.

GBATEK references:
[DS interrupts](https://problemkaputt.de/gbatek.htm#dsinterrupts) ·
[IPC](https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc)

---

## 7.1 The interrupt controller

Each CPU has its own IE / IF / IME triple:

```text
              ARM9                      ARM7
   IME   4000208h                  4000208h     master enable (bit 0)
   IE    4000210h                  4000200h     which sources are enabled
   IF    4000214h                  4000202h     which sources are pending

   An IRQ line is asserted when:      IME != 0  &&  (IE & IF) != 0
```

Lunaris holds one controller per CPU in an array indexed by `is_arm9`
([interrupt_controller.rs:16-42](core/src/hw/interrupt_controller.rs#L16-L42)):

```rust
pub struct InterruptController {
    pub enable: InterruptEnable,
    pub master_enable: InterruptMasterEnable,
    pub request: InterruptRequest,
}

impl InterruptController {
    // ...
    pub fn interrupts_requested(&self, ignore_ime: bool) -> bool {
        (ignore_ime || self.master_enable.bits() != 0)
            && (self.request.bits() & self.enable.bits()) != 0
    }
}
```

`ignore_ime` is a real hardware subtlety, not a shortcut: on the ARM7 a HALT is
released by an enabled interrupt **even when IME is clear**. The caller passes
`self.haltcnt.halted()` ([hw.rs:267-272](core/src/hw.rs#L267-L272)):

```rust
pub fn arm7_interrupts_requested(&mut self) -> bool {
    if unlikely(self.keypad.interrupt_requested()) {
        self.interrupts[0].request |= InterruptRequest::KEYPAD
    }
    self.interrupts[0].interrupts_requested(self.haltcnt.halted())
}
```

Keypad interrupts are folded in here rather than being pushed by the keypad,
because they are _level_ triggered on a button combination rather than edge
triggered by an event (Chapter 17).

### The source list

[interrupt_controller.rs:53-78](core/src/hw/interrupt_controller.rs#L53-L78):

```rust
    pub struct InterruptEnable: u32 {
        const VBLANK = 1 << 0;           // LCD V-Blank
        const HBLANK = 1 << 1;           // LCD H-Blank
        const VCOUNTER_MATCH = 1 << 2;   // LCD V-Counter match (DISPSTAT LYC)
        const TIMER0_OVERFLOW = 1 << 3;
        // ... TIMER1..3, SERIAL, DMA0..3 ...
        const KEYPAD = 1 << 12;          // Keypad – see KEYCNT for logic-AND/OR mode
        const GAME_PAK = 1 << 13;        // GBA cartridge slot (always 0 on retail NDS)
        // Bits 14-15: not used
        const IPC_SYNC = 1 << 16;              // IPCSYNC bit 14 sent by other CPU
        const IPC_SEND_FIFO_EMPTY = 1 << 17;   // Own send-FIFO transitioned to empty
        const IPC_RECV_FIFO_NOT_EMPTY = 1 << 18; // Other CPU's FIFO has data
        const GAME_CARD_TRANSFER_COMPLETION = 1 << 19; // NDS slot ROM block done
        const GAME_CARD_IREQ_MC = 1 << 20;     // NDS slot IREQ_MC line asserted
        const GEOMETRY_COMMAND_FIFO = 1 << 21; // 3-D GXFIFO below half-empty (ARM9 only)
        const LID_OPEN = 1 << 22;              // Lid/hinge open switch (ARM7 only)
        const SPI_BUS = 1 << 23;               // SPI bus transfer complete (ARM7 only)
        const WIFI = 1 << 24;                  // Wi-Fi (ARM7 only).
    }
```

```text
   bits 0-13   ── inherited unchanged from the GBA
   bits 14-15  ── unused
   bits 16-24  ── NDS additions: IPC, game card, 3-D FIFO, lid, SPI, Wi-Fi

   Which CPU sees which:
   ┌──────────────────────────┬──────┬──────┐
   │ source                   │ ARM9 │ ARM7 │
   ├──────────────────────────┼──────┼──────┤
   │ VBLANK / HBLANK / VMATCH │  ✓   │  ✓   │
   │ TIMER0-3, DMA0-3         │  ✓   │  ✓   │
   │ IPC_SYNC / FIFO          │  ✓   │  ✓   │
   │ GEOMETRY_COMMAND_FIFO    │  ✓   │  —   │
   │ LID_OPEN, SPI_BUS, WIFI  │  —   │  ✓   │
   │ KEYPAD                   │  ✓   │  ✓   │
   └──────────────────────────┴──────┴──────┘
```

> **Divergence noted in the source itself**
> ([interrupt_controller.rs:119](core/src/hw/interrupt_controller.rs#L119)):
> `// TODO: bit 21 (GEOMETRY_COMMAND_FIFO) is ARM9-only; ARM7 IF must never set it.`
> Lunaris shares one `InterruptRequest` bitflags type between both CPUs, so
> nothing structurally prevents an ARM7-side geometry bit. Nothing sets it
> today, so the effect is latent.

### IF is a write-1-to-clear register

The one thing about ARM interrupt controllers that catches everyone
([interrupt_controller.rs:196-209](core/src/hw/interrupt_controller.rs#L196-L209)):

```rust
    /// Acknowledges (clears) interrupt bits.
    ///
    /// Writing `1` to a bit clears that pending interrupt.
    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => self.bits &= !(value as u32),
            1 => self.bits &= !((value as u32) << 8),
            2 => self.bits &= !((value as u32) << 16),
            3 => self.bits &= !((value as u32) << 24),
            _ => unreachable!(),
        }
    }
```

```text
   Normal register:  write 1 → bit becomes 1
   IF register:      write 1 → bit becomes 0   ← acknowledge

   Get this backwards and the symptom is: the game takes its VBlank IRQ once,
   never clears it, and then either spins forever in the handler or never
   takes another interrupt at all.
```

### The full path of an interrupt

```text
   scheduler fires HBlank event
        │
   GPU handler:  self.interrupts[i].request |= InterruptRequest::HBLANK
        │
   ...CPU reaches its next instruction boundary...
        │
   ARM::emulate → handle_irq → arm9_interrupts_requested()
        │                            = IME && (IE & IF)
        ▼
   exception entry (Chapter 3 §3.6): mode swap, LR, PC = base|0x18
        │
   game's IRQ handler runs, reads IF, writes the same bits back
        ▼
   IF cleared, handler returns via SUBS PC, LR, #4
```

Interrupts are only ever _raised_ by setting a bit in `request`; nothing calls
into the CPU. That keeps the direction of control one-way and makes the
scheduler the only source of asynchrony.

---

## 7.2 IPC: how the two CPUs talk

Two independent mechanisms, both at `4000180h`+.

```text
   ┌─────────────────── IPCSYNC (4000180h) ────────────────────┐
   │                                                           │
   │   ARM9 side                        ARM7 side              │
   │   ┌──────────────┐                 ┌──────────────┐       │
   │   │ output[3:0]  │ ══════════════► │ input[3:0]   │       │
   │   │ input[3:0]   │ ◄══════════════ │ output[3:0]  │       │
   │   │ sync_irq     │                 │ sync_irq     │       │
   │   └──────────────┘                 └──────────────┘       │
   │       writing bit 13 pulses an IPC_SYNC IRQ on the other  │
   │       side, if that side has bit 14 (sync_irq) set        │
   └───────────────────────────────────────────────────────────┘

   ┌────────────── IPCFIFO (4000184h / 88h / 4100000h) ────────┐
   │                                                           │
   │   ARM9 send-FIFO (output9)      16 words × 32 bits        │
   │   ┌──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┐       │
   │   │  │  │  │  │  │  │  │  │  │  │  │  │  │  │  │  │ ═════►│ ARM7 reads
   │   └──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┘       │ 4100000h
   │                                                           │
   │   ARM7 send-FIFO (output7)                                │
   │   ┌──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┬──┐       │
   │◄══│  │  │  │  │  │  │  │  │  │  │  │  │  │  │  │  │       │ ARM9 reads
   │   └──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┘       │ 4100000h
   └───────────────────────────────────────────────────────────┘
```

State ([ipc.rs:31-49](core/src/hw/ipc.rs#L31-L49)):

```rust
pub struct IPC {
    fifocnt7: FIFOCNT,
    sync7: SYNC,
    /// ARM7 send-FIFO (ARM9 reads this via IPCFIFORECV at 4100000h).
    ///
    /// `VecDeque::load_in_place` does not consume the stored length prefix,
    /// so route through `Loadable` instead.
    #[load(with = "save.load()?", with_in_place = "*output7 = save.load()?")]
    output7: VecDeque<u32>,
    prev_value7: u32,
    fifocnt9: FIFOCNT,
    sync9: SYNC,
    /// ARM9 send-FIFO (ARM7 reads this via IPCFIFORECV at 4100000h).
    #[load(with = "save.load()?", with_in_place = "*output9 = save.load()?")]
    output9: VecDeque<u32>,
    prev_value9: u32,
}
```

(The `#[load(with = …)]` attribute on the `VecDeque`s is a workaround for a real
savestate bug — `load_in_place` did not consume the length prefix, corrupting
every field after it. Chapter 19.)

### IPCSYNC

A four-bit mailbox each way, typically used for a boot handshake before the
FIFO is enabled ([ipc.rs:237-256](core/src/hw/ipc.rs#L237-L256)):

```rust
    fn write(&mut self, other: &mut Self, byte: usize, value: u8) -> InterruptRequest {
        if match byte {
            0 => false,
            1 => {
                // IPCSYNC bits 8-11 only: bits 12-15 are the IRQ controls and
                // must not leak into the other CPU's 4-bit input field.
                self.output = value & 0xF;
                other.input = self.output;
                self.sync_irq = value >> 6 & 0x1 != 0;
                other.sync_irq && value >> 5 & 0x1 != 0
            }
```

Note `value & 0xF`. The comment records a bug class worth remembering: byte 1
of IPCSYNC carries _both_ the 4-bit data field and the two IRQ control bits, so
masking is mandatory or the IRQ bits appear as data on the other CPU.

```text
   IPCSYNC (16-bit)
    15 14 13 12 11    8 7      4 3      0
   ┌──┬──┬──┬──┬────────┬────────┬────────┐
   │ -│EN│IR│ -│ output │   -    │ input  │
   └──┴──┴──┴──┴────────┴────────┴────────┘
        │  │       │                  ▲
        │  │       └──────────────────┘ (to the OTHER CPU's input)
        │  └─ write 1: pulse IRQ at the other CPU
        └──── enable IPC_SYNC IRQ on THIS CPU
```

### The FIFOs

16 words each direction. Two rules govern the interrupts, and both are about
_transitions_, not levels.

**Send** ([ipc.rs:178-198](core/src/hw/ipc.rs#L178-L198)):

```rust
    fn send(
        send_cnt: &mut FIFOCNT,
        recv_cnt: &FIFOCNT,
        send_fifo: &mut VecDeque<u32>,
        value: u32,
    ) -> InterruptRequest {
        if !send_cnt.enable {
            return InterruptRequest::empty();
        }
        let interrupt =
            if recv_cnt.enable && recv_cnt.recv_fifo_not_empty_irq && send_fifo.is_empty() {
                InterruptRequest::IPC_RECV_FIFO_NOT_EMPTY
            } else {
                InterruptRequest::empty()
            };
        if send_fifo.len() == IPC::FIFO_LEN {
            send_cnt.error = true;
        } else {
            send_fifo.push_back(value);
        }
        interrupt
    }
```

`send_fifo.is_empty()` is checked **before** the push — the IRQ fires on the
empty→non-empty edge only. Overflow sets the error flag rather than dropping
silently or panicking.

**Receive** ([ipc.rs:143-169](core/src/hw/ipc.rs#L143-L169)):

```rust
        let interrupt = if let Some(value) = recv_fifo.pop_front() {
            *prev_value = value;
            if send_cnt.enable && send_cnt.send_fifo_empty_irq && recv_fifo.is_empty() {
                InterruptRequest::IPC_SEND_FIFO_EMPTY
            } else {
                InterruptRequest::empty()
            }
        } else {
            recv_cnt.error = true;
            InterruptRequest::empty()
        };
        (*prev_value, interrupt)
```

Reading an empty FIFO returns `prev_value` — the _last word received_ — and
sets the error flag. That is hardware behaviour, and it matters: a game polling
the FIFO without checking the empty flag must see a stable value, not zero.

The comment above that code records a fixed crash
([ipc.rs:152-156](core/src/hw/ipc.rs#L152-L156)):

```rust
        // Reading while the *sending* side has its FIFO disabled is legal on
        // hardware - the queue simply stays empty and the read returns the last
        // received word with the error flag set, which the empty branch below
        // already implements. This used to be an `assert!`, i.e. a crash on a
        // path the sound driver and the video decoder both exercise.
```

### Interrupt routing is crossed

The single most confusing part of IPC, and the source of many emulator bugs.
Look at where the interrupt goes ([mem.rs:48-74](core/src/hw/mem.rs#L48-L74)):

```rust
/// Reads one 32-bit word from the IPC receive-FIFO (IPCFIFORECV, 4100000h).
///
/// The interrupt is routed to the *sender*'s controller (not the reading CPU)
/// because the empty-IRQ notifies the sender that space became available.
fn ipc_fifo_recv(&mut self, is_arm9: bool) -> u32 {
    if is_arm9 {
        let (value, interrupt) = self.ipc.arm9_recv();
        self.interrupts[0].request |= interrupt;   // ← ARM7's controller
        value
    } else {
        let (value, interrupt) = self.ipc.arm7_recv();
        self.interrupts[1].request |= interrupt;   // ← ARM9's controller
        value
    }
}

/// Writes one 32-bit word to the IPC send-FIFO (IPCFIFOSEND, 4000188h).
///
/// Interrupt is routed to the *receiver*'s controller so it is woken when
/// data arrives.
fn ipc_fifo_send(&mut self, is_arm9: bool, value: u32) {
    if is_arm9 {
        self.interrupts[1].request |= self.ipc.arm7_send(value);
    } else {
        self.interrupts[0].request |= self.ipc.arm9_send(value);
    }
}
```

```text
   ARM9 writes a word            ARM7 drains the last word
        │                                  │
        │ IPC_RECV_FIFO_NOT_EMPTY          │ IPC_SEND_FIFO_EMPTY
        ▼                                  ▼
     ARM7's IF                          ARM9's IF
   "you have mail"                  "your outbox is empty"
```

Both interrupts cross to the _other_ CPU. `interrupts[0]` is ARM7 and
`interrupts[1]` is ARM9 ([hw.rs:110-111](core/src/hw.rs#L110-L111)), which is
worth keeping in mind while reading this code.

### IPCFIFOCNT

[ipc.rs:259-276](core/src/hw/ipc.rs#L259-L276) documents the layout, and
`get_fifo_status` derives the flags from the actual queue length rather than
tracking them separately ([ipc.rs:337-340](core/src/hw/ipc.rs#L337-L340)):

```rust
    fn get_fifo_status(fifo: &VecDeque<u32>) -> u8 {
        assert!(fifo.len() <= IPC::FIFO_LEN);
        ((fifo.len() == IPC::FIFO_LEN) as u8) << 1 | fifo.is_empty() as u8
    }
```

Deriving state instead of caching it removes an entire category of
desynchronisation bug — the same principle as the timer counter in Chapter 6.

```text
   IPCFIFOCNT
    15 14 13 12 11 10  9  8   7  6  5  4  3  2  1  0
   ┌──┬──┬──┬──┬──┬──┬──┬──┬ ─┬──┬──┬──┬──┬──┬──┬──┐
   │EN│ER│ -│ -│ -│ -│RN│RF│RE│ -│ -│ -│FL│SN│SF│SE│
   └──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┘
     │  │              │  │  │           │  │  │  └ send FIFO empty (RO)
     │  │              │  │  │           │  │  └─── send FIFO full  (RO)
     │  │              │  │  │           │  └────── send-empty IRQ enable
     │  │              │  │  │           └───────── flush send FIFO (WO)
     │  │              │  │  └───────────────────── recv FIFO empty (RO)
     │  │              │  └──────────────────────── recv FIFO full  (RO)
     │  │              └─────────────────────────── recv-not-empty IRQ enable
     │  └────────────────────────────────────────── error (write 1 to clear)
     └───────────────────────────────────────────── FIFO enable
```

---

## 7.3 Design notes

- **All IPC functions return an `InterruptRequest`** rather than touching the
  controllers. The caller decides where it lands. This is what makes the
  crossed routing above explicit and auditable in one place.
- **Errors are flags, not panics.** FIFO overflow and underflow set
  `FIFOCNT.error`; the earlier `assert!` version crashed on paths real games
  take.
- **Nothing here is scheduled.** IPC is entirely synchronous within a CPU's
  memory access. The 30-cycle slice cap in the main loop (Chapter 1) is what
  keeps the two CPUs' views of the FIFO plausibly ordered.

---

[← 6. The Scheduler and Timers](06_scheduler_and_timers.md) | [Next: 8. DMA →](08_dma.md)
