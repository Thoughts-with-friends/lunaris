//! DMA start triggers and the main memory display FIFO, driven through
//! the I/O registers like a game would.
use crate::emulator::Emulator;

/// DMAxCNT word: count, enable, word units, start timing (ARM9 bits 27-29).
const fn cnt9(count: u32, timing: u32, extra: u32) -> u32 {
    count | (1 << 31) | (1 << 26) | (timing << 27) | extra
}

fn emu() -> Emulator {
    let mut emu = Emulator::new();
    emu.power_on();
    emu
}

fn run_pending(emu: &mut Emulator) {
    if emu.dma_event.processing {
        emu.dma_handle_event();
    }
}

#[test]
fn vblank_dma_waits_for_vblank() {
    let mut emu = emu();
    emu.arm9_write_word(0x0200_0000, 0xCAFE_F00D);
    emu.arm9_write_word(0x040000B0, 0x0200_0000); // DMA0SAD
    emu.arm9_write_word(0x040000B4, 0x0200_1000); // DMA0DAD
    emu.arm9_write_word(0x040000B8, cnt9(1, 1, 0)); // VBlank
    run_pending(&mut emu);
    assert_eq!(emu.arm9_read_word(0x0200_1000), 0, "not before VBlank");

    emu.vblank_dma_request();
    run_pending(&mut emu);
    assert_eq!(emu.arm9_read_word(0x0200_1000), 0xCAFE_F00D);
    assert!(!emu.dma.dmas[0].cnt.enabled, "no repeat: disabled when done");
}

#[test]
fn arm7_timing_is_read_from_bits_12_13() {
    let mut emu = emu();
    emu.arm7_write_word(0x0200_0000, 0x1234_5678);
    emu.arm7_write_word(0x040000B0, 0x0200_0000);
    emu.arm7_write_word(0x040000B4, 0x0200_2000);
    // ARM7 VBlank: timing 1 in bits 28-29 of the word (12-13 of CNT_H).
    emu.arm7_write_word(0x040000B8, 1 | (1 << 31) | (1 << 26) | (1 << 28));
    assert_eq!(emu.dma.dmas[4].start_mode(), crate::dma::MODE7_VBLANK);
    run_pending(&mut emu);
    assert_eq!(emu.arm7_read_word(0x0200_2000), 0);
    emu.vblank_dma_request();
    run_pending(&mut emu);
    assert_eq!(emu.arm7_read_word(0x0200_2000), 0x1234_5678);
}

#[test]
fn two_channels_started_together_both_run() {
    let mut emu = emu();
    emu.arm9_write_word(0x0200_0000, 0x1111_1111);
    emu.arm9_write_word(0x0200_0004, 0x2222_2222);
    for (ch, src, dst) in [(0, 0x0200_0000, 0x0200_1000), (1, 0x0200_0004, 0x0200_1004)] {
        let base = 0x040000B0 + ch * 12;
        emu.arm9_write_word(base, src);
        emu.arm9_write_word(base + 4, dst);
        emu.arm9_write_word(base + 8, cnt9(1, 0, 0)); // immediate
    }
    run_pending(&mut emu);
    assert_eq!(emu.arm9_read_word(0x0200_1000), 0x1111_1111);
    assert_eq!(emu.arm9_read_word(0x0200_1004), 0x2222_2222);
    assert!(!emu.dma_active());
}

#[test]
fn arm9_count_uses_bits_16_to_20() {
    let mut emu = emu();
    emu.arm9_write_word(0x040000B8, 0x0001_0000 | 0x0002); // count bit 16 set
    assert_eq!(emu.dma.dmas[0].start_count(), 0x1_0002);
}

#[test]
fn main_memory_display_reads_the_fifo() {
    let mut emu = emu();
    for i in 0..256_u32 {
        emu.arm9_write_halfword(0x0200_0000 + i * 2, (i * 3) as u16);
    }
    emu.arm9_write_word(0x0400_0000, 3 << 16); // DISPCNT display mode 3
    emu.arm9_write_word(0x040000B0, 0x0200_0000);
    emu.arm9_write_word(0x040000B4, 0x0400_0068); // DISP_MMEM_FIFO
    // 4 words per request, repeat, fixed destination, timing 4.
    emu.arm9_write_word(0x040000B8, cnt9(4, 4, (1 << 25) | (2 << 21)));

    emu.fill_display_fifo_line();
    let line = emu.gpu.display_fifo.line;
    assert!(line.iter().enumerate().all(|(i, &p)| p == (i * 3) as u16));

    emu.vblank_dma_request();
    assert!(!emu.dma.dmas[0].cnt.enabled, "stopped at VBlank");
}
