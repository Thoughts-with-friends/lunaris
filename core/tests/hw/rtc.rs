//! Spec tests for `core/src/hw/rtc.rs` — the Seiko S-3511 real-time clock,
//! bit-banged by the ARM7 through register 4000138h.
//!
//! GBATEK "DS Real-Time Clock (RTC)": <https://problemkaputt.de/gbatek.htm#dsrealtimeclockrtc>

use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx},
    spec::Checks,
};

const RTC: u32 = 0x0400_0138;

/// Drives the three RTC lines. `out` = data line direction is output.
fn lines(hw: &mut crate::hw::HW, cs: bool, sck: bool, data: bool, out: bool) {
    let v = 0x40 | 0x20 | (out as u8) << 4 | (cs as u8) << 2 | (sck as u8) << 1 | data as u8;
    hw.arm7_write::<u8>(RTC, v);
}

/// One complete RTC transaction: command byte (MSB first, as Lunaris
/// expects), then `n` data bytes read LSB first.
fn rtc_read(hw: &mut crate::hw::HW, cmd: u8, n: usize) -> Vec<u8> {
    lines(hw, false, true, false, true);
    lines(hw, true, true, false, true);
    for bit in (0..8).rev() {
        let b = cmd >> bit & 1 != 0;
        lines(hw, true, false, b, true);
        lines(hw, true, true, b, true);
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let mut byte = 0u8;
        for bit in 0..8 {
            lines(hw, true, false, false, false);
            byte |= (hw.arm7_read::<u8>(RTC) & 1) << bit;
            lines(hw, true, true, false, false);
        }
        out.push(byte);
    }
    lines(hw, false, true, false, true);
    out
}

fn bcd(v: u8) -> Option<u32> {
    let (hi, lo) = (v >> 4, v & 0xF);
    (hi < 10 && lo < 10).then_some(hi as u32 * 10 + lo as u32)
}

#[test]
fn date_time_read_returns_seven_valid_bcd_bytes() {
    let mut nds = io_machine("rtc_datetime");
    let b = rtc_read(nds.hw_mut(), 0x65, 7);
    let year = bcd(b[0]).unwrap();
    let month = bcd(b[1]).unwrap();
    let day = bcd(b[2]).unwrap();
    let min = bcd(b[5]).unwrap();
    let sec = bcd(b[6]).unwrap();
    assert!(year < 100 && (1..=12).contains(&month) && (1..=31).contains(&day));
    assert!(b[3] < 7, "weekday 0..6");
    assert!(min < 60 && sec < 60);
}

#[test]
fn report() {
    let mut nds = io_machine("rtc_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/rtc.md", "Real-time clock (S-3511)", "hw/rtc.rs");
    doc.source("register-level test; date/time comes from the host clock (`chrono::Local::now`)");
    doc.bitfield(
        "4000138h RTC (ARM7)",
        8,
        &[
            (6, 6, "CS dir"),
            (5, 5, "SCK dir"),
            (4, 4, "SIO dir"),
            (2, 2, "CS"),
            (1, 1, "SCK"),
            (0, 0, "SIO"),
        ],
    );
    doc.code(
        "text",
        "CS   __/‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\\__\n\
         SCK  ‾‾‾‾\\_/‾\\_/‾\\_/‾ … ‾\\_/‾\\_/‾ … ‾\\_/‾‾‾\n\
         SIO      c7  c6  c5      c0  d0  d1 …  d7        (sampled on SCK falling edge)\n\
         \x20        └── command byte ──┘└ data bytes, LSB first ┘\n\
         \n\
         command = 0110 ppp r    ppp = parameter, r = 1 read / 0 write",
    );
    doc.table(
        &[("ppp", R), ("Parameter", L), ("Bytes", R), ("Lunaris `Parameter`", L)],
        &[
            vec!["0", "status register 1", "1", "`StatusReg1`"],
            vec!["1", "status register 2", "1", "`StatusReg2`"],
            vec!["2", "date + time (Y M D W h m s)", "7", "`DateTime(n)`"],
            vec!["3", "time (h m s)", "3", "`Time(n)`"],
            vec!["4", "alarm 1 / frequency duty", "3 / 1", "`Alarm1FreqDuty(n)`"],
            vec!["5", "alarm 2", "3", "`Alarm2(n)`"],
            vec!["6", "clock adjust", "1", "`ClockAdjust`"],
        ],
    );
    let mut c = Checks::new();
    let dt = rtc_read(hw, 0x65, 7);
    let names = ["year (20xx)", "month", "day", "weekday", "hour", "minute", "second"];
    let mut rows = Vec::new();
    for (i, n) in names.iter().enumerate() {
        let v = if i == 4 { dt[i] & 0x3F } else { dt[i] };
        rows.push(vec![
            i.to_string(),
            n.to_string(),
            hx(dt[i] as u64, 2),
            bcd(v).map(|x| x.to_string()).unwrap_or("invalid BCD".into()),
        ]);
        c.ok(&format!("{n} is BCD"), "each nibble 0-9", bcd(v).is_some(), hx(dt[i] as u64, 2));
    }
    doc.h2("Date/time read (command 65h)");
    doc.table(&[("Byte", R), ("Field", L), ("Raw", R), ("Decoded", R)], &rows);
    let s1 = rtc_read(hw, 0x61, 1)[0];
    doc.bitfield(
        "Status register 1",
        8,
        &[
            (7, 7, "RESET"),
            (6, 6, "INT2"),
            (5, 5, "INT1"),
            (4, 4, "SC1"),
            (3, 3, "SC0"),
            (2, 2, "BLD"),
            (1, 1, "12/24"),
            (0, 0, "POC"),
        ],
    );
    doc.p(&format!(
        "Status register 1 read back as {} (bit 1 = {} → {}-hour mode).",
        hx(s1 as u64, 2),
        s1 >> 1 & 1,
        if s1 & 2 != 0 { 24 } else { 12 }
    ));
    let hour_raw = dt[4];
    let now = chrono::Local::now();
    use chrono::Timelike as _;
    let expected_hour = if s1 & 2 != 0 { now.hour() } else { now.hour() % 12 };
    c.known(
        "hour encoding",
        "24h mode (status1 bit 1 = 1): hour 0-23 in BCD; 12h mode: 0-11 + bit 6 = PM",
        expected_hour,
        bcd(hour_raw & 0x3F).unwrap_or(99),
        "`DateTime::read` swaps the branches: `is_24h` returns `hour12()`",
    );
    doc.note("The hour check depends on the host clock: in 12-hour mode the swapped branch still yields the right number between 01:00 and 11:59, so this row can read ✅ in the morning and ⚠️ in the afternoon. The branch swap itself is visible in `DateTime::read`.");
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
