//! Spec tests for `core/src/hw/cartridge/key1_encryption.rs` — the KEY1
//! Blowfish variant keyed from BIOS7 and the game code.
//!
//! GBATEK "DS Encryption by Gamecode/Idcode (KEY1)":
//! <https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1>

use super::*;
use crate::test_support::{
    md::{Doc, L, R, hx},
    rom,
    spec::Checks,
};

fn key1(level: u32) -> Key1Encryption {
    let mut k = Key1Encryption::new(&free_bios::arm7::BIOS_ARM7_BIN);
    k.init_key_code(u32::from_le_bytes(*rom::layout::GAME_CODE), level, 2);
    k
}

#[test]
fn decrypt_inverts_encrypt_at_every_level() {
    for level in 1..=3 {
        let k = key1(level);
        for block in [[0u32, 0], [0x1234_5678, 0x9ABC_DEF0], [u32::MAX, 1]] {
            let mut b = block;
            k.encrypt(&mut b);
            assert_ne!(b, block, "level {level} changes the block");
            k.decrypt(&mut b);
            assert_eq!(b, block, "level {level} round trip");
        }
    }
}

#[test]
fn key_buffer_is_1048h_bytes_from_bios7_offset_30h() {
    assert_eq!(Key1Encryption::KEY_TABLE_SIZE * 4, 0x1048);
    let k = Key1Encryption::new(&free_bios::arm7::BIOS_ARM7_BIN);
    let first = u32::from_le_bytes(free_bios::arm7::BIOS_ARM7_BIN[0x30..0x34].try_into().unwrap());
    assert_eq!(k.original_key_buf[0], first);
}

#[test]
fn report() {
    let mut doc = Doc::new(
        "hw/cartridge/key1_encryption.md",
        "KEY1 encryption (Blowfish)",
        "hw/cartridge/key1_encryption.rs",
    );
    doc.source("free BIOS7 key table (`free_bios::arm7`) + synthetic game code `LNTS`");
    doc.p("KEY1 protects the early cartridge commands and the secure area. It is a 64-bit block Blowfish with 16 rounds whose 1048h-byte key buffer (18 P-array words + four 256-entry S-boxes) starts as a copy of BIOS7 30h..1077h and is then mixed with the game code.");
    doc.table(
        &[("Key buffer range", L), ("Words", R), ("Content", L), ("Lunaris const", L)],
        &[
            vec!["000h..047h", "18", "P-array (round keys P0..P17)", "`P_ARRAY_END = 44h/4`"],
            vec!["048h..447h", "256", "S-box 0", "`S_BOX0`"],
            vec!["448h..847h", "256", "S-box 1", "`S_BOX1`"],
            vec!["848h..C47h", "256", "S-box 2", "`S_BOX2`"],
            vec!["C48h..1047h", "256", "S-box 3", "`S_BOX3`"],
        ],
    );
    doc.code(
        "text",
        "init_key_code(idcode, level, modulo):\n\
         \x20 key_buf  = BIOS7[30h..1078h]\n\
         \x20 keycode  = [idcode, idcode/2, idcode*2]\n\
         \x20 level≥1: apply_keycode   ┐\n\
         \x20 level≥2: apply_keycode   ├─ each: encrypt keycode[1..3], keycode[0..2];\n\
         \x20 level≥3: keycode[1]*=2,  │        P[i] ^= bswap(keycode[i % modulo]);\n\
         \x20          keycode[2]/=2,  │        re-encrypt a zero block through the whole\n\
         \x20          apply_keycode   ┘        buffer to regenerate P and S\n\
         \n\
         round(i): z = P[i] ^ x\n\
         \x20         f = ((S0[z>>24] + S1[z>>16 & FF]) ^ S2[z>>8 & FF]) + S3[z & FF]\n\
         \x20         x' = f ^ y ;  y' = z\n\
         encrypt: rounds 0..15, out = (x ^ P16, y ^ P17)\n\
         decrypt: rounds 17..2, out = (x ^ P1,  y ^ P0)",
    );
    doc.table(
        &[("Level", R), ("Used for", L)],
        &[
            vec!["1", "(unused by Lunaris) KEY1 for firmware"],
            vec![
                "2",
                "cartridge KEY1 command mode (after command 3Ch); first 8 bytes of the secure area",
            ],
            vec!["3", "the whole 2 KiB secure area (`Cartridge::encrypt_secure_area`)"],
        ],
    );
    let mut c = Checks::new();
    let mut rows = Vec::new();
    for level in 1..=3 {
        let k = key1(level);
        let mut b = [0x6F72_6365u32, 0x6A62_4F79]; // "encryObj"
        k.encrypt(&mut b);
        let enc = b;
        k.decrypt(&mut b);
        c.eq(
            &format!("level {level} round trip"),
            "decrypt(encrypt(x)) = x",
            [0x6F72_6365u32, 0x6A62_4F79],
            b,
        );
        rows.push(vec![
            level.to_string(),
            "\"encryObj\"".into(),
            format!("{} {}", hx(enc[0] as u64, 8), hx(enc[1] as u64, 8)),
            hx(k.key_buf[0] as u64, 8),
        ]);
    }
    doc.h2("\"encryObj\" under each level (game code `LNTS`)");
    doc.table(&[("Level", R), ("Plain", L), ("Cipher words", L), ("P0 after keying", L)], &rows);
    c.eq("key buffer size", "1048h bytes", 0x1048, Key1Encryption::KEY_TABLE_SIZE * 4);
    doc.h2("Spec checks");
    c.write(&mut doc);
    doc.save();
    c.finish();
}
