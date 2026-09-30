//! Spec tests for `core/src/hw.rs` — direct boot: what the BIOS/firmware
//! would have left in memory before jumping to the game.
//!
//! GBATEK:
//! - "DS Cartridge Header" (header copy at 27FFE00h): <https://problemkaputt.de/gbatek.htm#dscartridgeheader>
//! - "DS Memory Maps" (boot area): <https://problemkaputt.de/gbatek.htm#dsmemorymaps>

use super::*;
use crate::test_support::{
    boot::boot,
    le16, le32,
    md::{Doc, L, R, hx},
    rom,
    spec::Checks,
};

/// The header fields direct boot depends on, copied out of the cartridge.
struct Bin {
    arm9_rom_offset: u32,
    arm9_ram_addr: u32,
    arm9_entry_addr: u32,
    arm9_size: u32,
    arm7_rom_offset: u32,
    arm7_ram_addr: u32,
    arm7_entry_addr: u32,
    arm7_size: u32,
}

fn bin(hw: &HW) -> Bin {
    let h = hw.cartridge.header();
    Bin {
        arm9_rom_offset: h.arm9_rom_offset,
        arm9_ram_addr: h.arm9_ram_addr,
        arm9_entry_addr: h.arm9_entry_addr,
        arm9_size: h.arm9_size,
        arm7_rom_offset: h.arm7_rom_offset,
        arm7_ram_addr: h.arm7_ram_addr,
        arm7_entry_addr: h.arm7_entry_addr,
        arm7_size: h.arm7_size,
    }
}

fn checks(hw: &mut HW, rom: &[u8], c: &mut Checks) {
    let h = bin(hw);
    let hdr: Vec<u8> = (0..0x170).map(|i| hw.arm9_read::<u8>(0x027F_FE00 + i)).collect();
    c.ok("header copy", "ROM 000h..16Fh at 27FFE00h", hdr == rom[..0x170], "0x170 bytes compared");
    for base in [0x027F_F800u32, 0x027F_FC00] {
        c.hex(
            &format!("chip ID @{base:08X}"),
            "chip ID (1st and 2nd copy)",
            hw.cartridge.chip_id(),
            hw.arm9_read::<u32>(base),
        );
        c.hex(
            &format!("header CRC @{:08X}", base + 8),
            "cartridge header CRC16",
            le16(rom, 0x15E),
            hw.arm9_read::<u16>(base + 8),
        );
        c.hex(
            &format!("secure CRC @{:08X}", base + 0xA),
            "secure area CRC16",
            le16(rom, 0x6C),
            hw.arm9_read::<u16>(base + 0xA),
        );
    }
    c.hex("27FF850h", "ARM7BIOS CRC (5835h)", 0x5835u16, hw.arm9_read::<u16>(0x027F_F850));
    c.hex("27FFC10h", "ARM7BIOS CRC copy (5835h)", 0x5835u16, hw.arm9_read::<u16>(0x027F_FC10));
    c.hex("27FFC40h", "boot indicator: 1 = cartridge", 1u16, hw.arm9_read::<u16>(0x027F_FC40));
    let arm9: Vec<u8> =
        (0..h.arm9_size.min(0x400)).map(|i| hw.arm9_read::<u8>(h.arm9_ram_addr + i)).collect();
    c.ok(
        "ARM9 binary",
        "copied from arm9_rom_offset to arm9_ram_addr",
        arm9 == rom[h.arm9_rom_offset as usize..][..arm9.len()],
        format!("{} bytes checked", arm9.len()),
    );
    let arm7: Vec<u8> =
        (0..h.arm7_size.min(0x400)).map(|i| hw.arm7_read::<u8>(h.arm7_ram_addr + i)).collect();
    c.ok(
        "ARM7 binary",
        "copied from arm7_rom_offset to arm7_ram_addr",
        arm7 == rom[h.arm7_rom_offset as usize..][..arm7.len()],
        format!("{} bytes checked", arm7.len()),
    );
    c.eq(
        "POSTFLG",
        "4000300h = 1 after boot (both CPUs)",
        (1, 1),
        (hw.arm9_read::<u8>(0x0400_0300), hw.arm7_read::<u8>(0x0400_0300)),
    );
}

#[test]
fn direct_boot_state_for_the_synthetic_rom() {
    let r = rom::synthetic();
    let mut nds = boot(r, "hw_boot_synth");
    let mut c = Checks::new();
    checks(nds.hw_mut(), &r.bytes, &mut c);
    c.finish();
}

#[test]
fn report() {
    let r = rom::test_rom();
    let mut nds = boot(r, "hw_boot_report");
    let (arm7, arm9) = nds.cpus();
    let (pc7, pc9, cpsr9) = (arm7.regs()[15], arm9.regs()[15], arm9.regs().cpsr());
    let hw = nds.hw_mut();
    let mut doc =
        Doc::new("hw.md", "Direct boot: memory state before the first instruction", "hw.rs");
    doc.source(&r.label());
    doc.p("Lunaris skips the BIOS and firmware menu (`direct_boot = true`). `HW::new` → `init_mem` and `ARM::new` → `init_arm9`/`init_arm7` recreate what the real boot chain leaves behind:");
    doc.code(
        "text",
        "cartridge ROM                         main RAM (4 MiB, 02000000h)\n\
         ┌──────────────┐  000h..16Fh ──────► 027FFE00h  header copy\n\
         │ header       │  chip ID / CRCs ──► 027FF800h, 027FFC00h  (two copies)\n\
         ├──────────────┤\n\
         │ ARM9 binary  │ ─────────────────► arm9_ram_addr  → ARM9 PC = arm9_entry_addr\n\
         │ ARM7 binary  │ ─────────────────► arm7_ram_addr  → ARM7 PC = arm7_entry_addr\n\
         └──────────────┘\n\
         both CPUs start in SVC mode, IRQ/FIQ disabled (CPSR = D3h); POSTFLG = 1",
    );
    let h = bin(hw);
    let area = |hw: &mut HW, a: u32| hx(hw.arm9_read::<u32>(a) as u64, 8);
    doc.table(
        &[("Address", L), ("Size", R), ("Meaning", L), ("Value", L)],
        &[
            vec!["027FF800h".into(), "4".into(), "chip ID (1st)".into(), area(hw, 0x027F_F800)],
            vec!["027FF804h".into(), "4".into(), "chip ID (2nd)".into(), area(hw, 0x027F_F804)],
            vec![
                "027FF808h".into(),
                "2".into(),
                "header CRC16".into(),
                hx(hw.arm9_read::<u16>(0x027F_F808) as u64, 4),
            ],
            vec![
                "027FF80Ah".into(),
                "2".into(),
                "secure-area CRC16".into(),
                hx(hw.arm9_read::<u16>(0x027F_F80A) as u64, 4),
            ],
            vec![
                "027FF850h".into(),
                "2".into(),
                "ARM7 BIOS CRC".into(),
                hx(hw.arm9_read::<u16>(0x027F_F850) as u64, 4),
            ],
            vec![
                "027FFC00h".into(),
                "0x10".into(),
                "2nd copy of 27FF800h..".into(),
                area(hw, 0x027F_FC00),
            ],
            vec![
                "027FFC40h".into(),
                "2".into(),
                "boot indicator (1 = cartridge)".into(),
                hx(hw.arm9_read::<u16>(0x027F_FC40) as u64, 4),
            ],
            vec![
                "023FFC80h".into(),
                "1".into(),
                "firmware version (Lunaris writes 5)".into(),
                hx(hw.arm9_read::<u8>(0x023F_FC80) as u64, 2),
            ],
            vec![
                "027FFE00h".into(),
                "0x170".into(),
                "cartridge header copy".into(),
                format!("`{}`…", crate::test_support::ascii(&r.bytes[..12])),
            ],
        ],
    );
    doc.table(
        &[("CPU", L), ("Binary", L), ("RAM", R), ("Entry", R), ("PC after boot", R)],
        &[
            vec![
                "ARM9".into(),
                format!("ROM {} + {}", hx(h.arm9_rom_offset as u64, 6), hx(h.arm9_size as u64, 6)),
                hx(h.arm9_ram_addr as u64, 8),
                hx(h.arm9_entry_addr as u64, 8),
                hx(pc9 as u64, 8),
            ],
            vec![
                "ARM7".into(),
                format!("ROM {} + {}", hx(h.arm7_rom_offset as u64, 6), hx(h.arm7_size as u64, 6)),
                hx(h.arm7_ram_addr as u64, 8),
                hx(h.arm7_entry_addr as u64, 8),
                hx(pc7 as u64, 8),
            ],
        ],
    );
    let mut c = Checks::new();
    checks(hw, &r.bytes, &mut c);
    c.hex(
        "ARM9 PC",
        "pipeline filled at arm9_entry_addr (PC = entry + 4)",
        h.arm9_entry_addr + 4,
        pc9,
    );
    c.hex(
        "ARM7 PC",
        "pipeline filled at arm7_entry_addr (PC = entry + 4)",
        h.arm7_entry_addr + 4,
        pc7,
    );
    c.hex("CPSR", "SVC mode, I and F set", 0xD3u32, cpsr9 & 0xFF);
    doc.h2("Spec checks");
    c.write(&mut doc);
    let _ = le32;
    doc.save();
    c.finish();
}
