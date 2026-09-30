//! Spec tests for `core/src/hw/ipc.rs` — IPCSYNC and the two 16-word IPC
//! FIFOs between the ARM9 and ARM7.
//!
//! GBATEK "DS Inter Process Communication (IPC)":
//! <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>

use crate::test_support::{
    boot::io_machine,
    md::{Doc, R, hx},
    spec::Checks,
};

const IPCSYNC: u32 = 0x0400_0180;
const IPCFIFOCNT: u32 = 0x0400_0184;
const IPCFIFOSEND: u32 = 0x0400_0188;
const IPCFIFORECV: u32 = 0x0410_0000;

#[test]
fn sync_output_of_one_cpu_is_the_input_of_the_other() {
    let mut nds = io_machine("ipc_sync");
    let hw = nds.hw_mut();
    hw.arm9_write::<u16>(IPCSYNC, 0x0A00); // ARM9 output = 0Ah
    assert_eq!(hw.arm7_read::<u16>(IPCSYNC) & 0xF, 0xA, "ARM7 input = ARM9 output");
    hw.arm7_write::<u16>(IPCSYNC, 0x0500);
    assert_eq!(hw.arm9_read::<u16>(IPCSYNC) & 0xF, 0x5);
}

#[test]
fn fifo_is_first_in_first_out_across_cpus() {
    let mut nds = io_machine("ipc_fifo");
    let hw = nds.hw_mut();
    hw.arm9_write::<u16>(IPCFIFOCNT, 0x8000);
    hw.arm7_write::<u16>(IPCFIFOCNT, 0x8000);
    for v in [0x1111_1111u32, 0x2222_2222, 0x3333_3333] {
        hw.arm9_write::<u32>(IPCFIFOSEND, v);
    }
    let got: Vec<u32> = (0..3).map(|_| hw.arm7_read::<u32>(IPCFIFORECV)).collect();
    assert_eq!(got, [0x1111_1111, 0x2222_2222, 0x3333_3333]);
}

#[test]
fn report() {
    let mut nds = io_machine("ipc_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/ipc.md", "Inter-processor communication (IPC)", "hw/ipc.rs");
    doc.source("register-level test (no ROM data involved)");
    doc.code(
        "text",
        "        ARM9                                         ARM7\n\
         \x20┌──────────────────┐   IPCSYNC out(4) ──► in(4)  ┌──────────────────┐\n\
         \x20│ 4000180h IPCSYNC │ ◄── in(4) ◄── out(4)         │ 4000180h IPCSYNC │\n\
         \x20│                  │   bit13 ──────► IRQ16 if bit14 set on receiver  │\n\
         \x20│ 4000188h SEND ───┼──► [16 × u32 FIFO] ───────────┼──► 4100000h RECV │\n\
         \x20│ 4100000h RECV ◄──┼─── [16 × u32 FIFO] ◄──────────┼─── 4000188h SEND │\n\
         \x20└──────────────────┘                              └──────────────────┘",
    );
    doc.bitfield(
        "4000180h IPCSYNC",
        16,
        &[(14, 14, "IRQEN"), (13, 13, "SEND IRQ"), (11, 8, "OUT"), (3, 0, "IN (=remote OUT)")],
    );
    doc.bitfield(
        "4000184h IPCFIFOCNT",
        16,
        &[
            (15, 15, "EN"),
            (14, 14, "ERR"),
            (10, 10, "RNE IRQ"),
            (9, 9, "R FULL"),
            (8, 8, "R EMPTY"),
            (3, 3, "CLR"),
            (2, 2, "SE IRQ"),
            (1, 1, "S FULL"),
            (0, 0, "S EMPTY"),
        ],
    );
    let mut c = Checks::new();
    c.hex("FIFOCNT reset", "send+recv empty = 0101h", 0x0101u16, hw.arm9_read::<u16>(IPCFIFOCNT));
    hw.arm9_write::<u16>(IPCSYNC, 0x0C00);
    c.eq("SYNC crossing", "ARM7 IN = ARM9 OUT", 0xC, hw.arm7_read::<u16>(IPCSYNC) & 0xF);
    hw.arm7_write::<u16>(IPCSYNC, 0x4000); // ARM7 enables sync IRQ
    hw.arm9_write::<u16>(IPCSYNC, 0x2000); // ARM9 sends IRQ
    c.eq(
        "SYNC IRQ",
        "remote bit 13 + local bit 14 → IF bit 16",
        true,
        hw.interrupts[0].request.bits() & (1 << 16) != 0,
    );

    hw.arm9_write::<u16>(IPCFIFOCNT, 0x8000);
    hw.arm7_write::<u16>(IPCFIFOCNT, 0x8000);
    let mut rows = Vec::new();
    for i in 0..17u32 {
        hw.arm9_write::<u32>(IPCFIFOSEND, 0xA000_0000 + i);
        let cnt9 = hw.arm9_read::<u16>(IPCFIFOCNT);
        let cnt7 = hw.arm7_read::<u16>(IPCFIFOCNT);
        if matches!(i, 0 | 1 | 14 | 15 | 16) {
            rows.push(vec![(i + 1).to_string(), hx(cnt9 as u64, 4), hx(cnt7 as u64, 4)]);
        }
    }
    doc.h2("Filling the ARM9→ARM7 FIFO");
    doc.table(&[("Words sent", R), ("ARM9 FIFOCNT", R), ("ARM7 FIFOCNT", R)], &rows);
    let cnt9 = hw.arm9_read::<u16>(IPCFIFOCNT);
    c.eq("send FIFO full", "16 words → ARM9 bit 1 (send full)", 1, cnt9 >> 1 & 1);
    c.eq("overflow error", "17th word → ARM9 bit 14 (error), word dropped", 1, cnt9 >> 14 & 1);
    c.eq("recv full", "ARM7 sees bit 9 (recv full)", 1, hw.arm7_read::<u16>(IPCFIFOCNT) >> 9 & 1);
    let first = hw.arm7_read::<u32>(IPCFIFORECV);
    c.hex("FIFO order", "first word out is the first word in", 0xA000_0000u32, first);
    for _ in 0..15 {
        hw.arm7_read::<u32>(IPCFIFORECV);
    }
    c.eq("drained", "ARM7 recv empty after 16 reads", 1, hw.arm7_read::<u16>(IPCFIFOCNT) >> 8 & 1);
    let again = hw.arm7_read::<u32>(IPCFIFORECV);
    c.hex("read empty", "reading an empty FIFO returns the last word", 0xA000_000Fu32, again);
    c.eq(
        "read-empty error",
        "…and sets ARM7 FIFOCNT bit 14",
        1,
        hw.arm7_read::<u16>(IPCFIFOCNT) >> 14 & 1,
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
