//! Spec tests for `core/src/hw/spi/tsc.rs` — the TSC2046 touchscreen ADC on
//! SPI device 2.
//!
//! GBATEK "DS Touch Screen Controller (TSC)":
//! <https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc>

use crate::{
    hw::spi::spec::spi,
    test_support::{
        boot::io_machine,
        md::{Doc, L, R, hx},
        spec::Checks,
    },
};

/// Control byte: start bit 7, channel bits 4-6, 12-bit mode, differential.
const fn ctrl(channel: u8) -> u8 {
    0x80 | channel << 4
}

/// Converts channel `ch` and returns the 12-bit result (MSB first over
/// the two bytes clocked after the control byte).
fn sample(hw: &mut crate::hw::HW, ch: u8) -> u16 {
    let r = spi(hw, 2, &[ctrl(ch), 0, 0]);
    (r[1] as u16) << 5 | (r[2] as u16) >> 3
}

#[test]
fn touch_position_is_reported_as_pixel_times_16() {
    let mut nds = io_machine("tsc_touch");
    let hw = nds.hw_mut();
    hw.press_screen(100, 50);
    assert_eq!(sample(hw, 5), 100 << 4, "channel 5 = X");
    assert_eq!(sample(hw, 1), 50 << 4, "channel 1 = Y");
    hw.release_screen();
    assert_eq!(sample(hw, 1), 0xFFF, "released: Y reads FFFh");
}

#[test]
fn report() {
    let mut nds = io_machine("tsc_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/spi/tsc.md", "Touchscreen controller (TSC2046)", "hw/spi/tsc.rs");
    doc.source("register-level test through SPICNT/SPIDATA (device 2)");
    doc.bitfield(
        "TSC control byte",
        8,
        &[(7, 7, "START"), (6, 4, "CHANNEL"), (3, 3, "8BIT"), (2, 2, "SER/DFR"), (1, 0, "POWER")],
    );
    doc.code(
        "text",
        "SPIDATA write:  [ctrl]      [00h]            [00h]\n\
         SPIDATA read:   (junk)      0 D11..D5        D4..D0 000\n\
         result = byte1 << 5 | byte2 >> 3            (TSC::write keeps `pos`)",
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for (x, y) in [(0usize, 0usize), (128, 96), (255, 191)] {
        hw.press_screen(x, y);
        let (sx, sy) = (sample(hw, 5), sample(hw, 1));
        c.eq(
            &format!("touch ({x},{y})"),
            "ADC = pixel << 4 (identity calibration)",
            ((x as u16) << 4, (y as u16) << 4),
            (sx, sy),
        );
        rows.push(vec![format!("({x}, {y})"), hx(sx as u64, 3), hx(sy as u64, 3)]);
    }
    hw.release_screen();
    rows.push(vec!["released".into(), hx(sample(hw, 5) as u64, 3), hx(sample(hw, 1) as u64, 3)]);
    c.eq("released Y", "no touch → Y = FFFh", 0xFFF, sample(hw, 1));
    doc.table(
        &[("Channel", R), ("Measures", L), ("Lunaris", L)],
        &[
            vec!["1", "Y position", "`TSC::y`"],
            vec!["5", "X position", "`TSC::x`"],
            vec!["6", "microphone (AUX)", "always 0 (TODO)"],
            vec!["0,2,3,4,7", "temperature / pressure / battery", "FFFh"],
        ],
    );
    doc.h2("Observed samples");
    doc.table(&[("Touch", L), ("X (ch 5)", R), ("Y (ch 1)", R)], &rows);
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
