//! Spec tests for `core/src/hw/spu.rs` — sound channel registers and the
//! IMA-ADPCM decoder.
//!
//! GBATEK:
//! - "DS Sound Channels 0..15": <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
//! - "DS Sound Notes" (ADPCM pseudo-code): <https://problemkaputt.de/gbatek.htm#dssoundnotes>

use super::*;
use crate::test_support::{
    boot::io_machine,
    md::{Doc, L, R, hx},
    png::{BLACK, BLUE, Canvas, GRAY, RED, WHITE},
    spec::Checks,
};

/// GBATEK reference ADPCM decoder (one nibble), with its ±7FFFh clamp.
fn ref_decode(value: &mut i32, index: &mut i32, nibble: u8) {
    let t = SPU::ADPCM_TABLE[*index as usize] as i32;
    let mut diff = t / 8;
    if nibble & 1 != 0 {
        diff += t / 4;
    }
    if nibble & 2 != 0 {
        diff += t / 2;
    }
    if nibble & 4 != 0 {
        diff += t;
    }
    *value =
        if nibble & 8 != 0 { (*value - diff).max(-0x7FFF) } else { (*value + diff).min(0x7FFF) };
    *index = (*index + SPU::ADPCM_INDEX_TABLE[(nibble & 7) as usize]).clamp(0, 88);
}

/// Minimal IMA-ADPCM encoder (greedy, uses the reference decoder to track
/// state) producing the nibble stream for `samples`.
fn encode(samples: &[i16]) -> (u32, Vec<u8>) {
    let (mut v, mut idx) = (samples[0] as i32, 0);
    let header = (v as u16 as u32) | (idx as u32) << 16;
    let mut nibbles = Vec::new();
    for &s in &samples[1..] {
        let best = (0..16u8)
            .min_by_key(|&n| {
                let (mut tv, mut ti) = (v, idx);
                ref_decode(&mut tv, &mut ti, n);
                (tv - s as i32).abs()
            })
            .unwrap();
        ref_decode(&mut v, &mut idx, best);
        nibbles.push(best);
    }
    let bytes = nibbles.chunks(2).map(|p| p[0] | p.get(1).copied().unwrap_or(0) << 4).collect();
    (header, bytes)
}

fn decode_with_core(ch: &mut Channel<impl ChannelType>, header: u32, bytes: &[u8]) -> Vec<i16> {
    ch.set_initial_adpcm(header);
    ch.adpcm_low_nibble = true;
    let mut out = Vec::new();
    for &b in bytes {
        ch.set_adpcm_data(b);
        out.push(ch.sample());
        ch.set_adpcm_data(b);
        out.push(ch.sample());
    }
    out
}

fn sine(n: usize) -> Vec<i16> {
    (0..n)
        .map(|i| ((i as f64 / 24.0).sin() * 12000.0 + (i as f64 / 7.0).sin() * 3000.0) as i16)
        .collect()
}

#[test]
fn adpcm_decoder_matches_gbatek_reference() {
    let mut nds = io_machine("spu_adpcm");
    let hw = nds.hw_mut();
    let src = sine(257);
    let (header, bytes) = encode(&src);
    let got = decode_with_core(&mut hw.spu.base_channels[0], header, &bytes);
    let (mut v, mut i) = (header as u16 as i16 as i32, (header >> 16) as i32);
    for (k, &b) in bytes.iter().enumerate() {
        for (j, n) in [b & 0xF, b >> 4].into_iter().enumerate() {
            ref_decode(&mut v, &mut i, n);
            assert_eq!(got[k * 2 + j] as i32, v, "sample {}", k * 2 + j);
        }
    }
}

#[test]
fn report() {
    let mut nds = io_machine("spu_report");
    let hw = nds.hw_mut();
    let mut doc = Doc::new("hw/spu.md", "Sound (SPU): channels and ADPCM", "hw/spu.rs");
    doc.source("register-level test on the ARM7 bus + direct decoder calls");
    doc.table(
        &[("Channels", L), ("Formats", L), ("Lunaris", L)],
        &[
            vec!["0–7", "PCM8, PCM16, IMA-ADPCM", "`SPU::base_channels`"],
            vec!["8–13", "…plus PSG square wave (8 duty cycles)", "`SPU::psg_channels`"],
            vec!["14–15", "…plus white noise (15-bit LFSR)", "`SPU::noise_channels`"],
        ],
    );
    doc.bitfield(
        "40004x0h SOUNDxCNT",
        32,
        &[
            (31, 31, "START"),
            (30, 29, "FORMAT"),
            (28, 27, "REPEAT"),
            (26, 24, "DUTY"),
            (22, 16, "PAN"),
            (15, 15, "HOLD"),
            (9, 8, "DIV"),
            (6, 0, "VOLUME"),
        ],
    );
    doc.table(
        &[("Offset", L), ("Name", L), ("Meaning", L)],
        &[
            vec!["+0", "SOUNDxCNT", "control (above)"],
            vec!["+4", "SOUNDxSAD", "source address (W)"],
            vec!["+8", "SOUNDxTMR", "sample rate: 33.51 MHz / 2 / (10000h − TMR)"],
            vec!["+A", "SOUNDxPNT", "loop start in words"],
            vec!["+C", "SOUNDxLEN", "length in words"],
        ],
    );
    doc.code(
        "text",
        "ADPCM stream: [header u32][byte][byte]…   header = value (s16, bits 0-15) | index (bits 16-22)\n\
         each byte = two nibbles, LOW nibble first\n\
         nibble d: t = ADPCM_TABLE[index]\n\
         \x20         diff = t/8 + (d&1 ? t/4) + (d&2 ? t/2) + (d&4 ? t)\n\
         \x20         value = d&8 ? max(value−diff, −7FFFh) : min(value+diff, +7FFFh)\n\
         \x20         index = clamp(index + INDEX_TABLE[d&7], 0, 88)",
    );
    let mut c = Checks::new();
    c.eq("ADPCM_TABLE length", "89 entries", 89, SPU::ADPCM_TABLE.len());
    c.eq(
        "ADPCM_TABLE ends",
        "first 7, last 32767",
        (7, 32767),
        (SPU::ADPCM_TABLE[0], SPU::ADPCM_TABLE[88]),
    );
    c.eq(
        "INDEX_TABLE",
        "−1,−1,−1,−1,2,4,6,8",
        [-1, -1, -1, -1, 2, 4, 6, 8],
        SPU::ADPCM_INDEX_TABLE,
    );

    let src = sine(512);
    let (header, bytes) = encode(&src);
    let got = decode_with_core(&mut hw.spu.base_channels[0], header, &bytes);
    let (mut v, mut i) = (header as u16 as i16 as i32, (header >> 16) as i32);
    let mut mismatch = 0;
    let mut reference = Vec::new();
    for &b in &bytes {
        for n in [b & 0xF, b >> 4] {
            ref_decode(&mut v, &mut i, n);
            reference.push(v);
        }
    }
    for (a, b) in got.iter().zip(&reference) {
        if *a as i32 != *b {
            mismatch += 1;
        }
    }
    c.eq("decoder vs reference", "every decoded sample equals the GBATEK pseudo-code", 0, mismatch);
    let (w, h) = (1024usize, 220usize);
    let mut img = Canvas::new(w, h, WHITE);
    img.line(0, h as i64 / 2, w as i64, h as i64 / 2, GRAY);
    let y = |s: i32| (h as i32 / 2 - s * (h as i32 / 2 - 10) / 16000) as i64;
    for k in 1..src.len() {
        let x0 = ((k - 1) * 2) as i64;
        img.line(x0, y(src[k - 1] as i32), x0 + 2, y(src[k] as i32), BLUE);
    }
    // got[j] is the decode of source sample j + 1 (sample 0 is the header).
    for j in 1..got.len().min(src.len() - 1) {
        let x0 = (j * 2) as i64;
        img.line(x0, y(got[j - 1] as i32), x0 + 2, y(got[j] as i32), RED);
    }
    img.text(6, 6, "BLUE: SOURCE PCM16   RED: CORE ADPCM DECODE", 2, BLACK);
    img.save("hw/spu/adpcm_waveform.png");
    doc.h2("ADPCM round trip");
    doc.p(&format!("A 512-sample PCM16 test signal is encoded to IMA-ADPCM by the test (header {}), fed nibble by nibble into `Channel::set_adpcm_data`, and compared against the GBATEK pseudo-code:", hx(header as u64, 8)));
    doc.image("waveform", "spu/adpcm_waveform.png");
    doc.hexdump(0, &bytes[..48]);

    // Saturation edge: GBATEK clamps to ±7FFFh.
    let ch = &mut hw.spu.base_channels[1];
    ch.set_initial_adpcm(88 << 16 | (-0x7FF0i16 as u16 as u32));
    ch.adpcm_low_nibble = true;
    ch.set_adpcm_data(0x0F);
    const WRAP: &str = "`set_adpcm_data` casts `diff` (up to 61436 at index 88) to i16 before the saturating add/sub, so it wraps negative and the sample moves the wrong way; the clamp limit is also i16::MIN (−8000h) instead of −7FFFh";
    c.known(
        "negative clamp",
        "value = max(value − diff, −7FFFh)",
        -0x7FFF,
        ch.sample() as i32,
        WRAP,
    );
    ch.set_initial_adpcm(88 << 16 | 0x7FF0);
    ch.adpcm_low_nibble = true;
    ch.set_adpcm_data(0x07);
    c.known(
        "positive clamp",
        "value = min(value + diff, +7FFFh)",
        0x7FFF,
        ch.sample() as i32,
        WRAP,
    );

    hw.arm7_write::<u32>(0x0400_0400, 0x0040_007F);
    c.hex(
        "SOUND0CNT read-back",
        "volume 127, pan 64 (no START)",
        0x0040_007Fu32,
        hw.arm7_read::<u32>(0x0400_0400),
    );
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
    let _ = (L, R);
}
