//! Cycle-accurate event scheduler.
//!
//! Hardware events (HBlank, VBlank, DMA, timer overflows, …) are stored in a
//! min-heap ([`PriorityQueue`] with [`Reverse`] priorities) keyed by the cycle
//! at which they should fire.  Each tick the CPU advances to a target cycle and
//! [`HW::handle_events`] drains any events that are due.

use std::{
    cmp::{Eq, PartialEq, Reverse},
    hash::Hash,
};

use priority_queue::PriorityQueue;

use super::{HW, spu};

/// Callback type invoked when a scheduled event fires.
pub type EventHandler = fn(&mut HW, Event);

impl HW {
    /// Advances the clock to `new_cycle` and fires all events that are due.
    pub fn handle_events(&mut self, new_cycle: usize) {
        assert!(self.scheduler.cycle <= new_cycle);
        self.scheduler.cycle = new_cycle;
        while let Some(wrapper) = self.scheduler.get_next_event() {
            (wrapper.handler)(self, wrapper.event);
        }
    }

    /// Advances the clock to the next scheduled event and fires it immediately.
    ///
    /// Used when the 3-D bus is stalled and the CPUs cannot run ahead.
    pub fn clock_until_event(&mut self) {
        let (_, Reverse(cycle)) = self.scheduler.event_queue.peek().unwrap();
        if self.scheduler.cycle > *cycle {
            return;
        }
        let (wrapper, Reverse(cycle)) = self.scheduler.event_queue.pop().unwrap();
        self.scheduler.cycle = cycle;
        (wrapper.handler)(self, wrapper.event);
    }

    pub fn cycle(&self) -> usize {
        self.scheduler.cycle
    }

    /// Returns the cycle at which the next event will fire, or the current
    /// cycle if that event is already overdue.
    pub fn cycle_at_next_event(&self) -> usize {
        let (_wrapper, Reverse(cycle)) = self.scheduler.event_queue.peek().unwrap();
        if self.scheduler.cycle > *cycle { self.scheduler.cycle } else { *cycle }
    }

    fn dummy_handler(&mut self, _event: Event) {
        unreachable!()
    }
}

/// Priority-queue–based event scheduler.
///
/// During savestate serialization the queue is flattened into two parallel
/// `Vec`s (`pending_event_types` / `pending_fire_cycles`) because function
/// pointers cannot be serialized.  [`restore_events`](Scheduler::restore_events)
/// rebuilds the live queue in `HW::post_load_hw`.
pub struct Scheduler {
    pub cycle: usize,
    event_queue: PriorityQueue<EventWrapper, Reverse<usize>>,
    // Populated during load, consumed by HW::post_load_hw to rebuild event_queue.
    pending_event_types: Vec<Event>,
    pending_fire_cycles: Vec<usize>,
}

// `cycle` and `fire_cycles` are absolute master-clock cycle counters. They are
// stored as `u64` because emu-utils serializes `usize` as `u32`, which
// silently truncates after ~128s of real play.
// See `docs/design/savestate-and-video-design.md` §3.
impl emu_utils::Storable for Scheduler {
    fn store<S: emu_utils::WriteSavestate>(&mut self, save: &mut S) -> Result<(), S::Error> {
        let mut event_types: Vec<Event> = Vec::with_capacity(self.event_queue.len());
        let mut fire_cycles: Vec<u64> = Vec::with_capacity(self.event_queue.len());
        for (wrapper, Reverse(cycle)) in &self.event_queue {
            event_types.push(wrapper.event);
            fire_cycles.push(*cycle as u64);
        }
        let mut cycle_u64 = self.cycle as u64;
        save.start_struct()?;
        save.start_field(b"cycle")?;
        save.store(&mut cycle_u64)?;
        save.start_field(b"event_types")?;
        save.store(&mut event_types)?;
        save.start_field(b"fire_cycles")?;
        save.store(&mut fire_cycles)?;
        save.end_struct()?;
        Ok(())
    }
}

impl emu_utils::Loadable for Scheduler {
    fn load<S: emu_utils::ReadSavestate>(save: &mut S) -> Result<Self, S::Error> {
        save.start_struct()?;
        save.start_field(b"cycle")?;
        let cycle = save.load::<u64>()? as usize;
        save.start_field(b"event_types")?;
        let pending_event_types = save.load::<Vec<Event>>()?;
        save.start_field(b"fire_cycles")?;
        let pending_fire_cycles =
            save.load::<Vec<u64>>()?.into_iter().map(|c| c as usize).collect();
        save.end_struct()?;
        Ok(Scheduler {
            cycle,
            event_queue: PriorityQueue::new(),
            pending_event_types,
            pending_fire_cycles,
        })
    }
}

impl emu_utils::LoadableInPlace for Scheduler {
    fn load_in_place<S: emu_utils::ReadSavestate>(&mut self, save: &mut S) -> Result<(), S::Error> {
        save.start_struct()?;
        save.start_field(b"cycle")?;
        self.cycle = save.load::<u64>()? as usize;
        save.start_field(b"event_types")?;
        self.pending_event_types = save.load::<Vec<Event>>()?;
        save.start_field(b"fire_cycles")?;
        self.pending_fire_cycles =
            save.load::<Vec<u64>>()?.into_iter().map(|c| c as usize).collect();
        self.event_queue.clear();
        save.end_struct()?;
        Ok(())
    }
}

impl Scheduler {
    pub fn new() -> Scheduler {
        let queue = PriorityQueue::new();
        Scheduler {
            cycle: 0,
            event_queue: queue,
            pending_event_types: Vec::new(),
            pending_fire_cycles: Vec::new(),
        }
    }

    /// Rebuilds the live event queue from the deserialized parallel vectors.
    ///
    /// `handler_fn` maps each [`Event`] variant back to its [`EventHandler`].
    pub fn restore_events(&mut self, handler_fn: fn(&Event) -> EventHandler) {
        for (event, fire_cycle) in
            self.pending_event_types.drain(..).zip(self.pending_fire_cycles.drain(..))
        {
            let handler = handler_fn(&event);
            self.event_queue.push(EventWrapper { event, handler }, Reverse(fire_cycle));
        }
    }

    /// Test-only: shifts `cycle` and every queued event's fire cycle by
    /// `offset`, simulating a long play session for u32-overflow regression
    /// tests. See `docs/design/savestate-and-video-design.md` §3.4.
    #[cfg(test)]
    pub(crate) fn offset_cycle_for_test(&mut self, offset: usize) {
        self.cycle = self.cycle.wrapping_add(offset);
        let mut shifted = Vec::with_capacity(self.event_queue.len());
        while let Some((wrapper, Reverse(cycle))) = self.event_queue.pop() {
            shifted.push((wrapper, cycle.wrapping_add(offset)));
        }
        for (wrapper, cycle) in shifted {
            self.event_queue.push(wrapper, Reverse(cycle));
        }
    }

    fn get_next_event(&mut self) -> Option<EventWrapper> {
        // There should always be at least one event in the queue
        let (_event_type, Reverse(cycle)) = self.event_queue.peek().unwrap();
        if self.cycle >= *cycle { Some(self.event_queue.pop().unwrap().0) } else { None }
    }

    pub fn schedule(&mut self, event: Event, handler: EventHandler, delay: usize) {
        let wrapper = EventWrapper::new(event, handler);
        self.event_queue.push(wrapper, Reverse(self.cycle + delay));
    }

    /// Schedules `event` to fire on the current cycle (no delay).
    pub fn run_now(&mut self, event: Event, handler: EventHandler) {
        self.schedule(event, handler, 0);
    }

    pub fn remove(&mut self, event: Event) {
        let wrapper = EventWrapper::new(event, HW::dummy_handler);
        self.event_queue.remove(&wrapper);
    }
}

/// All events that can be scheduled on the emulator timeline.
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

struct EventWrapper {
    event: Event,
    handler: EventHandler,
}

impl EventWrapper {
    pub fn new(event: Event, handler: EventHandler) -> Self {
        EventWrapper { event, handler }
    }
}

impl PartialEq for EventWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.event.eq(&other.event)
    }
}

impl Eq for EventWrapper {}

impl Hash for EventWrapper {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.event.hash(state);
    }
}
