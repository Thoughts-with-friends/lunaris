//! The FPGA as the CPUs see it: through `Emulator`'s bus, EXMEMCNT-gated.
#![expect(clippy::expect_used, reason = "test setup")]
use ds_emu::{CpuType, Emulator};
use ds_fpga::FpgaSlot;

fn emulator_with_timer() -> Emulator {
    let mut emu = Emulator::new();
    let mut fpga = FpgaSlot::default();
    fpga.add_spec(ds_fpga::example_timer()).expect("bundled spec");
    emu.slot2.insert(Some(Box::new(fpga)));
    emu
}

#[test]
fn arm9_loads_and_stores_reach_the_block() {
    let mut emu = emulator_with_timer();
    assert_eq!(emu.read_word(0x0800_0000, CpuType::Arm9), 0x4650_4741);
    emu.write_word(0x0800_0018, 0xCAFE_F00D, CpuType::Arm9); // SCRATCH
    assert_eq!(emu.read_word(0x0800_0018, CpuType::Arm9), 0xCAFE_F00D);
    assert_eq!(emu.read_halfword(0x0800_001A, CpuType::Arm9), 0xCAFE);
    assert_eq!(emu.read_byte(0x0800_0018, CpuType::Arm9), 0x0D);
}

#[test]
fn exmemcnt_hands_the_slot_to_the_arm7() {
    let mut emu = emulator_with_timer();
    emu.ex_mem_cnt |= 1 << 7;
    assert_eq!(emu.read_word(0x0800_0000, CpuType::Arm7), 0x4650_4741);
    assert_ne!(emu.read_word(0x0800_0000, CpuType::Arm9), 0x4650_4741, "ARM9 lost the slot");
}

#[test]
fn the_device_survives_a_savestate_load() {
    let mut emu = emulator_with_timer();
    let state = emu.save_state().expect("state");
    emu.load_state(&state).expect("load");
    assert_eq!(emu.read_word(0x0800_0000, CpuType::Arm9), 0x4650_4741);
}

/// The ARM9 loads a program into the RISC-V sample, starts it and reads
/// the result back, all with ordinary loads and stores.
#[test]
fn arm9_drives_the_riscv_sample() {
    use ds_fpga::samples::{fibonacci, riscv_map as map, riscv_rv32i};
    let mut emu = Emulator::new();
    let mut fpga = FpgaSlot::default();
    fpga.add_spec(riscv_rv32i()).expect("bundled sample");
    emu.slot2.insert(Some(Box::new(fpga)));
    let base = 0x0800_0000;
    for (i, word) in fibonacci(10).iter().enumerate() {
        emu.write_word(base + map::IMEM + 4 * i as u32, *word, CpuType::Arm9);
    }
    emu.write_word(base + map::IRQ_EN, 0b01, CpuType::Arm9);
    emu.write_word(base + map::CTRL, map::RESET_AND_RUN, CpuType::Arm9);
    let mut irq = false;
    for _ in 0..1_000 {
        irq |= emu.slot2.tick(1024 * 16);
    }
    assert_eq!(emu.read_word(base + map::MAILBOX_OUT, CpuType::Arm9), 55);
    assert_eq!(emu.read_word(base + map::STATUS, CpuType::Arm9) & map::HALTED, map::HALTED);
    assert!(irq, "halting raised the Slot-2 interrupt");
}
