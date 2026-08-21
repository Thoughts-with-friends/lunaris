# 6. The Scheduler and Timers

Everything in the DS that happens "later" — the next scanline, a DMA step, a
timer overflow, an audio sample — goes through one min-heap. This chapter is
the beating heart of the emulator's timing model.

GBATEK references:
[DS timers](https://problemkaputt.de/gbatek.htm#dstimers) ·
[GBA timers (register layout, prescaler, count-up)](https://problemkaputt.de/gbatek.htm#gbatimers)

---

## 6.1 The scheduler in one picture

```text
   HW.scheduler
   ┌───────────────────────────────────────────────────────────────┐
   │  cycle: 41_337_190          ← "now", in master clock cycles   │
   │                                                               │
   │  event_queue: PriorityQueue<EventWrapper, Reverse<usize>>     │
   │                                                               │
   │      fire cycle   event                       handler         │
   │      ──────────   ────────────────────────    ──────────────  │
   │      41_337_204   GenerateAudioSample     →   HW::on_audio…   │
   │      41_337_512   HBlank                  →   HW::on_hblank   │
   │      41_338_000   TimerOverflow(true, 1)  →   HW::on_timer…   │
   │      41_339_100   Wifi                    →   HW::on_wifi…    │
   │      41_400_000   ROMBlockEnded(true)     →   HW::on_rom…     │
   │           ▲                                                   │
   │           └─ min-heap: peek() is always the soonest           │
   └───────────────────────────────────────────────────────────────┘
```

`Reverse<usize>` turns the max-heap of `priority_queue` into a min-heap; the
smallest fire-cycle is the highest priority
([scheduler.rs:59-71](core/src/hw/scheduler.rs#L59-L71)).

---

## 6.2 Advancing time

Two entry points, matching the two branches of the main loop (Chapter 1).

**Normal path** — the CPUs already ran to `new_cycle`, now drain what came due
([scheduler.rs:20-28](core/src/hw/scheduler.rs#L20-L28)):

```rust
pub fn handle_events(&mut self, new_cycle: usize) {
    assert!(self.scheduler.cycle <= new_cycle);
    self.scheduler.cycle = new_cycle;
    while let Some(wrapper) = self.scheduler.get_next_event() {
        (wrapper.handler)(self, wrapper.event);
    }
}
```

**Stalled path** — the CPUs cannot run, so jump straight to the next event
([scheduler.rs:30-41](core/src/hw/scheduler.rs#L30-L41)):

```rust
pub fn clock_until_event(&mut self) {
    let (_, Reverse(cycle)) = self.scheduler.event_queue.peek().unwrap();
    if self.scheduler.cycle > *cycle {
        return;
    }
    let (wrapper, Reverse(cycle)) = self.scheduler.event_queue.pop().unwrap();
    self.scheduler.cycle = cycle;
    (wrapper.handler)(self, wrapper.event);
}
```

```text
   Normal                              Stalled (3-D FIFO full)
   ──────                              ───────────────────────
   cycle ──►──►──►──►──► target        cycle ─────────────────► next event
        CPUs execute here                   no CPU execution at all
        then events fire                    one event fires, CPUs re-synced
```

`get_next_event` returns `None` as soon as the head of the heap is in the
future, so `handle_events` naturally stops at the right point
([scheduler.rs:172-176](core/src/hw/scheduler.rs#L172-L176)):

```rust
fn get_next_event(&mut self) -> Option<EventWrapper> {
    // There should always be at least one event in the queue
    let (_event_type, Reverse(cycle)) = self.event_queue.peek().unwrap();
    if self.cycle >= *cycle { Some(self.event_queue.pop().unwrap().0) } else { None }
}
```

Note the `unwrap()` and its comment: the queue is **never empty**, because the
GPU always has a scanline event pending. That invariant is load-bearing — the
main loop calls `cycle_at_next_event()` unconditionally.

---

## 6.3 The event set

Every asynchronous thing the hardware does is one variant
([scheduler.rs:194-222](core/src/hw/scheduler.rs#L194-L222)):

```rust
pub enum Event {
    /// DMA transfer step: `(is_nds9, channel_index)`.
    DMA(bool, usize),
    /// GPU: begin rendering the next scanline.
    StartNextLine,
    /// GPU: start of horizontal blanking period.
    HBlank,
    /// GPU: start of vertical blanking period (frame complete).
    VBlank,
    /// 3-D engine: check whether the geometry command FIFO has space for DMA.
    CheckGeometryCommandFIFO,
    /// Timer overflow: `(is_nds9, timer_index)`.
    TimerOverflow(bool, usize),
    /// Cartridge: one 4-byte word was transferred from the ROM bus.
    ROMWordTransfered(bool),
    /// Cartridge: the current ROM data block transfer finished.
    ROMBlockEnded(bool),
    /// SPU: mix one audio output sample.
    GenerateAudioSample,
    /// SPU: advance one audio channel by one step.
    StepAudioChannel(spu::ChannelSpec),
    /// SPU: reset an audio channel after its sample finishes.
    ResetAudioChannel(spu::ChannelSpec),
    /// Wi-Fi: one 8 microsecond hardware tick. See `core/src/hw/wifi/mod.rs`.
    Wifi,
}
```

The `bool` in `DMA(bool, usize)` and friends is `is_nds9` — one enum covers
both CPUs' copies of a peripheral.

### Identity vs. priority

`EventWrapper` hashes and compares on the **event only**, not the handler
([scheduler.rs:235-247](core/src/hw/scheduler.rs#L235-L247)):

```rust
impl PartialEq for EventWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.event.eq(&other.event)
    }
}
```

That is what makes cancellation work: `remove` constructs a wrapper with a
dummy handler and the queue still finds it
([scheduler.rs:188-191](core/src/hw/scheduler.rs#L188-L191)):

```rust
pub fn remove(&mut self, event: Event) {
    let wrapper = EventWrapper::new(event, HW::dummy_handler);
    self.event_queue.remove(&wrapper);
}
```

The corollary: **an event variant can only be queued once at a time.** Pushing
`TimerOverflow(true, 1)` twice replaces rather than duplicates. Every event
variant is therefore designed to carry enough identity (which CPU, which
channel) to be unique.

---

## 6.4 Savestates: function pointers cannot be serialised

`EventHandler` is `fn(&mut HW, Event)`. A raw function address is meaningless
across runs. Lunaris flattens the queue into two parallel vectors on save and
rebuilds it on load ([scheduler.rs:77-96](core/src/hw/scheduler.rs#L77-L96)):

```rust
impl emu_utils::Storable for Scheduler {
    fn store<S: emu_utils::WriteSavestate>(&mut self, save: &mut S) -> Result<(), S::Error> {
        let mut event_types: Vec<Event> = Vec::with_capacity(self.event_queue.len());
        let mut fire_cycles: Vec<u64> = Vec::with_capacity(self.event_queue.len());
        for (wrapper, Reverse(cycle)) in &self.event_queue {
            event_types.push(wrapper.event);
            fire_cycles.push(*cycle as u64);
        }
```

and on the way back in ([scheduler.rs:145-155](core/src/hw/scheduler.rs#L145-L155)):

```rust
pub fn restore_events(&mut self, handler_fn: fn(&Event) -> EventHandler) {
    for (event, fire_cycle) in
        self.pending_event_types.drain(..).zip(self.pending_fire_cycles.drain(..))
    {
        let handler = handler_fn(&event);
        self.event_queue.push(EventWrapper { event, handler }, Reverse(fire_cycle));
    }
}
```

```text
   save                              load
   ────                              ────
   heap ──flatten──► [Event]         [Event] ──map──► handler fn
                     [u64 cycle]     [u64]           │
                          │                          ▼
                          ▼                    push into fresh heap
                    savestate bytes             (HW::post_load_hw)
```

`handler_fn` is the `Event → EventHandler` mapping supplied by `HW`, which is
the one place that must stay in sync with the `Event` enum. Chapter 19 covers
the whole savestate flow.

Note the explicit `as u64` casts: cycle counters overflow `u32` in about two
minutes (Chapter 2, §2.4).

---

## 6.5 Hardware timers

Four per CPU, 16-bit, up-counting, at `4000100h`–`400010Fh`.

```text
   TMxCNT_L (write) = reload value
   TMxCNT_L (read)  = current counter

   TMxCNT_H
    15        7    6     3   2      1   0
   ┌──────┬───┬───────┬─────┬─────────────┐
   │  -   │ST │   -   │ IRQ │ CU │  PS    │
   └──────┴───┴───────┴─────┴────┴────────┘
             │           │     │      └── prescaler: 0=÷1 1=÷64 2=÷256 3=÷1024
             │           │     └───────── count-up (cascade), not on timer 0
             │           └─────────────── IRQ on overflow
             └─────────────────────────── start/stop

   counting                     overflow
   ────────                     ────────
   0xFFFE → 0xFFFF → reload     ─► IRQ (if enabled)
                                 ─► clock the next timer (if it is count-up)
```

### The key trick: timers are not ticked

A per-cycle timer update would cost eight increments per master cycle. Instead
Lunaris **derives** the counter from elapsed time when it is read, and
pre-schedules exactly one overflow event
([timers.rs:81-93](core/src/hw/timers.rs#L81-L93)):

```rust
/// Two counting modes (selected by TMCNT_H bit 2):
///
/// - **Regular**: counter is derived on-the-fly from `(global_cycle - start_cycle)`
///   so reads are O(1) without per-cycle updates.  An overflow event is
///   pre-scheduled in the [`Scheduler`] at construction.
///
/// - **Count-up (cascade)**: counter incremented explicitly by the previous
///   timer's overflow handler.  No scheduler event is used.
```

The derivation ([timers.rs:156-167](core/src/hw/timers.rs#L156-L167)):

```rust
fn calc_counter(&self, global_cycle: usize) -> u16 {
    let cycles_passed = global_cycle as i64 - self.start_cycle as i64; // Avoid underflow
    // Counter stores the reload value
    if cycles_passed >= self.time_till_first_clock as i64 {
        let cycles_passed = cycles_passed as usize; // Cast back to usize for division
        let cycles_passed = cycles_passed - self.time_till_first_clock;
        let counter_change = cycles_passed / Timers::PRESCALERS[self.cnt.prescaler as usize];
        assert!(counter_change < 0x1_0000);
        self.counter + 1 + counter_change as u16
    } else {
        self.counter
    }
}
```

```text
   start_cycle
        │◄─ time_till_first_clock ─►│◄─ prescaler ─►│◄─ prescaler ─►│
        │                           │               │               │
   ─────┴───────────────────────────┼───────────────┼───────────────┼──►
                                    ▲               ▲               ▲
                              counter+1       counter+2       counter+3

        │◄──────────────── timer_len = prescaler × (0x10000 − reload − 1) ──►│
                                                                             ▲
                                                                        overflow
```

`time_till_first_clock` exists because a timer started mid-prescaler-period
must align to the _global_ prescaler phase, not to its own start
([timers.rs:174-201](core/src/hw/timers.rs#L174-L201)):

```rust
pub fn create_event(&mut self, scheduler: &mut Scheduler, delay: usize) {
    self.start_cycle = scheduler.cycle + delay;
    // Syncs prescaler to global cycle
    let prescaler = Timers::PRESCALERS[self.cnt.prescaler as usize];
    // ...
    // Add 1 for 1 cycle delay in timer start
    self.time_till_first_clock = prescaler - (self.start_cycle + 1) % prescaler;
    self.timer_len = prescaler * (0x10000 - self.reload as usize - 1);
    scheduler.schedule(
        Event::TimerOverflow(self.is_nds9, self.index),
        HW::on_timer_overflow,
        delay + self.time_till_first_clock + self.timer_len,
    );
}
```

The `+ 1` is GBATEK's documented one-cycle delay when the start bit goes 0→1.

### Starting, stopping, and reprogramming

Writing TMxCNT_H is the delicate part
([timers.rs:233-256](core/src/hw/timers.rs#L233-L256)):

```rust
        2 => {
            // ...
            scheduler.remove(Event::TimerOverflow(self.is_nds9, self.index));
            let prev_start = self.cnt.start;
            if !self.is_count_up() && self.cnt.start {
                self.counter = self.calc_counter(global_cycle);
            }
            self.cnt.write(scheduler, 0, value);
            if !self.is_count_up() {
                if !prev_start && self.cnt.start {
                    self.reload();
                    self.create_event(scheduler, 1);
                } else if self.cnt.start {
                    self.create_event(scheduler, 0);
                }
            } else {
```

Read that as a state machine:

```text
   write to TMxCNT_H
        │
        ├─ cancel any pending overflow event         (always)
        ├─ freeze the derived counter into `counter` (if it was running)
        ├─ apply the new control bits
        └─ regular timer?
              ├─ stopped → started : reload, schedule with 1-cycle delay
              ├─ still running     : reschedule with the new prescaler
              └─ stopped           : nothing (counter stays frozen)
           count-up timer?
              └─ stopped → started : counter = reload, no event
```

Freezing the counter _before_ applying the new bits is what makes a
prescaler change mid-count correct.

### Cascade

Timer _n_ overflowing clocks timer _n+1_ if that one is in count-up mode, and
that can cascade recursively ([timers.rs:274-295](core/src/hw/timers.rs#L274-L295)):

```rust
pub fn on_timer_overflow(&mut self, event: Event) {
    let (is_nds9, num) = match event {
        Event::TimerOverflow(is_nds9, num) => (is_nds9, num),
        _ => unreachable!(),
    };
    let i = is_nds9 as usize;
    if self.timers[i][num].cnt.irq {
        self.interrupts[i].request |= self.timers[i].timers[num].interrupt
    }
    // Cascade Timers
    if num + 1 < Timers::NUM_TIMERS
        && self.timers[i][num + 1].is_count_up()
        && self.timers[i][num + 1].clock()
    {
        self.on_timer_overflow(Event::TimerOverflow(is_nds9, num + 1))
    }
    // TODO: Can I move this up to avoid recreating timers
    if !self.timers[i][num].is_count_up() {
        self.timers[i][num].reload();
        self.timers[i][num].create_event(&mut self.scheduler, 0);
    }
}
```

```text
   Timer0 (÷1024, reload 0xFC00)          16-bit, prescaled
        │ overflow every ~1024 × 1024 cycles
        ▼
   Timer1 (count-up)  ──► +1 per Timer0 overflow
        │ overflow
        ▼
   Timer2 (count-up)  ──► +1 per Timer1 overflow
                            = a 48-bit timer built from three 16-bit ones
```

Recursion depth is bounded by four, so no loop guard is needed.

### Prescaler rates

From the module docs ([timers.rs:6-14](core/src/hw/timers.rs#L6-L14)):

| Value | Divisor | ARM9 effective rate | ARM7 effective rate |
| ----- | ------- | ------------------- | ------------------- |
| 0     | 1       | ~66.233 MHz         | ~33.513 MHz         |
| 1     | 64      | ~1034 kHz           | ~523 kHz            |
| 2     | 256     | ~258 kHz            | ~131 kHz            |
| 3     | 1024    | ~64.7 kHz           | ~32.7 kHz           |

---

## 6.6 Design notes and divergences

- **`assert!(counter_change < 0x1_0000)` in `calc_counter`.** If a scheduled
  overflow were ever missed, the derived counter would wrap and the assert
  fires instead of silently corrupting state. It is a cheap invariant check on
  the whole timing model.
- **Timer events are scheduled against master cycles for both CPUs.** The ARM9
  prescaler table above reflects its 2× clock, but timer events themselves live
  on the single master timeline.
- **Timer read granularity.** Reads are exact because they are derived; there is
  no "timer updated only at event boundaries" error.
- **No `Event` for the RTC.** The RTC is polled through SPI rather than
  scheduled (Chapter 17).

---

[← 5. Memory Map and Page Tables](05_memory_map.md) | [Next: 7. Interrupts and IPC →](07_interrupts_and_ipc.md)
