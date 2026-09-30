//! Spec tests for `core/src/hw/scheduler.rs` — the event min-heap that
//! drives every timed device.

use std::cell::RefCell;

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R},
    spec::Checks,
};

thread_local! {
    static LOG: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

fn logger(hw: &mut HW, e: Event) {
    if let Event::ROMWordTransfered(tag) | Event::ROMBlockEnded(tag) = e {
        LOG.with(|l| l.borrow_mut().push((tag as usize, hw.cycle())));
    }
}

#[test]
fn events_fire_in_due_order() {
    let mut nds = io_machine("sched_order");
    let hw = nds.hw_mut();
    LOG.with(|l| l.borrow_mut().clear());
    let t0 = hw.cycle();
    hw.scheduler.schedule(Event::ROMWordTransfered(true), logger, 50);
    hw.scheduler.schedule(Event::ROMBlockEnded(false), logger, 10);
    for c in (t0 + 1)..=(t0 + 60) {
        hw.clock_until(c);
    }
    let log = LOG.with(|l| l.borrow().clone());
    assert_eq!(log, vec![(0, t0 + 10), (1, t0 + 50)]);
}

#[test]
fn report() {
    let mut nds = io_machine("sched_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/scheduler.md", "Event scheduler", "hw/scheduler.rs");
    doc.source("direct `Scheduler::schedule` calls on a fresh machine");
    doc.code(
        "text",
        "PriorityQueue<EventWrapper, Reverse<cycle>>   (min-heap keyed by due cycle)\n\
         \n\
         schedule(event, handler, delay) -> due = cycle + delay\n\
         \x20   (scheduling an Event that is already queued replaces the old entry)\n\
         clock_until(t): cycle = t; pop every event with due <= t, call handler(hw, event)\n\
         NDS::emulate_frame: t advances <= 30 cycles at a time (or to the next due event)",
    );
    let mut c = Checks::new();
    LOG.with(|l| l.borrow_mut().clear());
    let t0 = hw.cycle();
    hw.scheduler.schedule(Event::ROMWordTransfered(true), logger, 50);
    hw.scheduler.schedule(Event::ROMBlockEnded(false), logger, 10);
    hw.scheduler.schedule(Event::ROMBlockEnded(true), logger, 30);
    hw.clock_until(t0 + 100);
    let coarse = LOG.with(|l| l.borrow().clone());
    c.eq(
        "order",
        "handlers run in due-cycle order",
        vec![0usize, 1, 1],
        coarse.iter().map(|e| e.0).collect::<Vec<_>>(),
    );
    c.eq(
        "handler cycle (event-accurate stepping)",
        "stepping to each due cycle, a handler sees exactly its due cycle",
        true,
        {
            let mut n = io_machine("sched_exact");
            let h = n.hw_mut();
            LOG.with(|l| l.borrow_mut().clear());
            let t = h.cycle();
            h.scheduler.schedule(Event::ROMBlockEnded(false), logger, 10);
            h.scheduler.schedule(Event::ROMWordTransfered(true), logger, 50);
            while h.cycle() < t + 60 {
                let next = h.cycle_at_next_event().clamp(h.cycle() + 1, t + 60);
                h.clock_until(next);
            }
            LOG.with(|l| l.borrow().clone()) == vec![(0, t + 10), (1, t + 50)]
        },
    );
    doc.note("API contract: `handle_events` sets `cycle = target` before popping, so a handler observes the cycle passed to `clock_until`. `NDS::emulate_frame` always passes `min(cycle + 30, cycle_at_next_event())`, i.e. never jumps past a due event, so in emulation every handler runs at its exact due cycle. The coarse single-call table below only illustrates the contract.");
    doc.table(
        &[("Event", L), ("Scheduled +", R), ("Observed cycle (one clock_until(+100))", R)],
        &coarse
            .iter()
            .zip([10, 30, 50])
            .map(|(e, d)| vec![format!("tag {}", e.0), d.to_string(), format!("+{}", e.1 - t0)])
            .collect::<Vec<_>>(),
    );
    hw.scheduler.schedule(Event::ROMBlockEnded(false), logger, 10);
    hw.scheduler.schedule(Event::ROMBlockEnded(false), logger, 20);
    LOG.with(|l| l.borrow_mut().clear());
    let t1 = hw.cycle();
    hw.clock_until(t1 + 40);
    c.eq(
        "re-schedule",
        "the same Event scheduled twice keeps one entry",
        1,
        LOG.with(|l| l.borrow().len()),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
