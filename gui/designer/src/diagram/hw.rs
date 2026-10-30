//! The DS as a block design: every hardware block, and every net between
//! two block ports. Ports are not listed separately: a block's input and
//! output ports are exactly the net ends that name it.
//!
//! ```text
//!  ARM9 --AHB--> ARM9 Interconnect --> Main RAM / WRAM / VRAM / 2D A,B /
//!  DMA9 --AHB-->        |              3D GX / timers / math / IPC / slot1 /
//!                       |              Slot-2 (FPGA) / keypad / IRQ9
//!  ARM7 --AHB--> ARM7 Interconnect --> Main RAM / WRAM / ARM7 WRAM / SPU /
//!  DMA7 --AHB-->                       SPI / RTC / Wi-Fi / timers / IPC ...
//!  sources --IRQ--> IRQ9 / IRQ7 --nIRQ--> ARM9 / ARM7
//!  VRAM -> 2D A/B, 3D -> 2D A BG0 -> LCDs;  SPU -> speakers;  SPI -> TSC...
//! ```
//!
//! Positions are the default layout in canvas units; the user's own layout
//! is saved separately (see `super::Layout`).

/// Colour group of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Cpu,
    Interconnect,
    Memory,
    Video,
    Audio,
    Io,
    Expansion,
}

pub struct BlockDef {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub category: Category,
    pub pos: (f32, f32),
    pub description: &'static str,
}

/// What a net carries (drawing style and colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    /// A memory-mapped bus (drawn thick, Vivado "interface" style).
    Bus,
    Irq,
    Video,
    Audio,
    Signal,
}

/// Which CPU's IF/IE an interrupt net lands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cpu {
    Arm9,
    Arm7,
}

pub struct NetDef {
    pub name: &'static str,
    pub from: (&'static str, &'static str),
    pub to: (&'static str, &'static str),
    pub kind: NetKind,
    /// For interrupt nets: the IF/IE bits the line sets.
    pub irq: Option<(Cpu, u32)>,
}

const fn block(
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    category: Category,
    pos: (f32, f32),
    description: &'static str,
) -> BlockDef {
    BlockDef { id, title, subtitle, category, pos, description }
}

pub const BLOCKS: &[BlockDef] = &[
    // ARM9 side.
    block(
        "irq9",
        "IRQ Controller 9",
        "IME / IE / IF",
        Category::Cpu,
        (0.0, 0.0),
        "Collects the ARM9 interrupt sources; the ARM9 takes an IRQ while IME and IE & IF are set.",
    ),
    block(
        "arm9",
        "ARM946E-S",
        "ARM9 @ 67 MHz, CP15, TCM",
        Category::Cpu,
        (0.0, 170.0),
        "Main CPU: game logic, 2D/3D graphics setup. 32 KiB ITCM, 16 KiB DTCM, caches and protection unit (CP15).",
    ),
    block(
        "dma9",
        "DMA9",
        "4 channels",
        Category::Cpu,
        (0.0, 330.0),
        "ARM9 DMA: immediate, V/HBlank, display, card, GX FIFO start modes.",
    ),
    block(
        "timers9",
        "Timers 9",
        "TM0-TM3",
        Category::Io,
        (0.0, 480.0),
        "Four 16-bit timers with prescalers and count-up chaining.",
    ),
    block(
        "math",
        "Math Unit",
        "DIV 64/32, SQRT",
        Category::Io,
        (0.0, 610.0),
        "Hardware divider and square root.",
    ),
    // ARM7 side.
    block(
        "irq7",
        "IRQ Controller 7",
        "IME / IE / IF",
        Category::Cpu,
        (0.0, 760.0),
        "Collects the ARM7 interrupt sources.",
    ),
    block(
        "arm7",
        "ARM7TDMI",
        "ARM7 @ 33 MHz",
        Category::Cpu,
        (0.0, 920.0),
        "Sub CPU: sound, touch screen, RTC, Wi-Fi, firmware, power management.",
    ),
    block(
        "dma7",
        "DMA7",
        "4 channels",
        Category::Cpu,
        (0.0, 1080.0),
        "ARM7 DMA: immediate, VBlank, card, Wi-Fi start modes.",
    ),
    block(
        "timers7",
        "Timers 7",
        "TM0-TM3",
        Category::Io,
        (0.0, 1230.0),
        "Four 16-bit timers of the ARM7.",
    ),
    // Interconnects.
    block(
        "ic9",
        "ARM9 Interconnect",
        "bus matrix",
        Category::Interconnect,
        (340.0, 90.0),
        "Address decoder of the ARM9 bus (GBATEK 'DS Memory Maps').",
    ),
    block(
        "ipc",
        "IPC",
        "SYNC + 2x16-word FIFO",
        Category::Io,
        (340.0, 640.0),
        "Inter-processor communication between the ARM9 and the ARM7.",
    ),
    block(
        "ic7",
        "ARM7 Interconnect",
        "bus matrix",
        Category::Interconnect,
        (340.0, 840.0),
        "Address decoder of the ARM7 bus.",
    ),
    // Memories and peripherals.
    block(
        "main_ram",
        "Main RAM",
        "4 MiB PSRAM",
        Category::Memory,
        (700.0, 0.0),
        "Shared main memory at 02000000h.",
    ),
    block(
        "wram",
        "Shared WRAM",
        "32 KiB (WRAMCNT)",
        Category::Memory,
        (700.0, 120.0),
        "Split between the CPUs by WRAMCNT.",
    ),
    block(
        "vram",
        "VRAM",
        "banks A-I, 656 KiB",
        Category::Memory,
        (700.0, 240.0),
        "Nine banks mapped by VRAMCNT_A..I to 2D BG/OBJ, 3D texture/palette, LCDC or ARM7.",
    ),
    block(
        "slot1",
        "Slot-1",
        "game card",
        Category::Io,
        (700.0, 420.0),
        "NDS cartridge bus (KEY1/KEY2) and backup memory over AUXSPI.",
    ),
    block(
        "slot2",
        "Slot-2 / FPGA",
        "GBA slot expansion",
        Category::Expansion,
        (700.0, 560.0),
        "08000000h..0A00FFFFh. Hosts the user FPGA (core/fpga): blocks described once and generated as Verilog, SystemVerilog and Rust.",
    ),
    block(
        "keypad",
        "Keypad",
        "KEYINPUT / EXTKEYIN",
        Category::Io,
        (700.0, 700.0),
        "Buttons; X/Y, hinge and pen-down via the ARM7.",
    ),
    block(
        "wram7",
        "ARM7 WRAM",
        "64 KiB",
        Category::Memory,
        (700.0, 840.0),
        "ARM7-only work RAM at 03800000h.",
    ),
    block(
        "spu",
        "SPU",
        "16 ch + 2 capture",
        Category::Audio,
        (700.0, 950.0),
        "Sound unit: PCM8/16, IMA-ADPCM, PSG and noise channels, mixer and capture.",
    ),
    block(
        "spi",
        "SPI Bus",
        "SPICNT / SPIDATA",
        Category::Io,
        (700.0, 1080.0),
        "Serial bus to the power manager, firmware flash and touch screen controller.",
    ),
    block(
        "rtc",
        "RTC",
        "S-3511A",
        Category::Io,
        (700.0, 1230.0),
        "Real-time clock on a 3-wire serial bus.",
    ),
    block(
        "wifi",
        "Wi-Fi",
        "Mitsumi MM3218",
        Category::Io,
        (700.0, 1340.0),
        "802.11b controller at 04800000h with its own 8 KiB RAM.",
    ),
    // Video and audio outputs.
    block(
        "ge3d",
        "3D Geometry",
        "GX FIFO, matrices",
        Category::Video,
        (1060.0, 60.0),
        "Geometry engine: commands, matrix stacks, lighting, clipping.",
    ),
    block(
        "re3d",
        "3D Rendering",
        "rasterizer",
        Category::Video,
        (1060.0, 200.0),
        "Scanline renderer; its output becomes BG0 of engine A.",
    ),
    block(
        "eng2d_a",
        "2D Engine A",
        "BG0-3, OBJ, capture",
        Category::Video,
        (1060.0, 340.0),
        "Main 2D engine (with 3D layer and display capture).",
    ),
    block(
        "eng2d_b",
        "2D Engine B",
        "BG0-3, OBJ",
        Category::Video,
        (1060.0, 480.0),
        "Sub 2D engine.",
    ),
    block(
        "speaker",
        "Speakers",
        "stereo",
        Category::Audio,
        (1060.0, 950.0),
        "The SPU mix as heard: waveform and spectrogram.",
    ),
    block(
        "pmic",
        "Power Manager",
        "SPI device 0",
        Category::Io,
        (1060.0, 1060.0),
        "Backlights, speaker amp, power LED.",
    ),
    block(
        "firmware",
        "Firmware",
        "SPI device 1, 256 KiB",
        Category::Memory,
        (1060.0, 1160.0),
        "User settings and Wi-Fi calibration.",
    ),
    block(
        "tsc",
        "Touch Screen",
        "TSC2046, SPI device 2",
        Category::Io,
        (1060.0, 1260.0),
        "Resistive touch screen and microphone ADC.",
    ),
    block(
        "lcd_top",
        "Top LCD",
        "256x192",
        Category::Video,
        (1400.0, 340.0),
        "Shows engine A, or B when POWCNT1 bit 15 is clear.",
    ),
    block(
        "lcd_bottom",
        "Bottom LCD",
        "256x192 + touch",
        Category::Video,
        (1400.0, 480.0),
        "Shows the other engine.",
    ),
];

const fn net(
    name: &'static str,
    from: (&'static str, &'static str),
    to: (&'static str, &'static str),
    kind: NetKind,
) -> NetDef {
    NetDef { name, from, to, kind, irq: None }
}

const fn irq(
    name: &'static str,
    from: (&'static str, &'static str),
    to: (&'static str, &'static str),
    cpu: Cpu,
    bits: u32,
) -> NetDef {
    NetDef { name, from, to, kind: NetKind::Irq, irq: Some((cpu, bits)) }
}

use Cpu::{Arm7, Arm9};
use NetKind::{Audio, Bus, Signal, Video};

pub const NETS: &[NetDef] = &[
    // ARM9 bus.
    net("arm9_ahb", ("arm9", "M_AHB"), ("ic9", "S00_CPU"), Bus),
    net("dma9_ahb", ("dma9", "M_AHB"), ("ic9", "S01_DMA"), Bus),
    net("ic9_main", ("ic9", "M_MAIN"), ("main_ram", "S_ARM9"), Bus),
    net("ic9_wram", ("ic9", "M_WRAM"), ("wram", "S_ARM9"), Bus),
    net("ic9_vram", ("ic9", "M_VRAM"), ("vram", "S_ARM9"), Bus),
    net("ic9_slot1", ("ic9", "M_SLOT1"), ("slot1", "S_ARM9"), Bus),
    net("ic9_slot2", ("ic9", "M_SLOT2"), ("slot2", "S_BUS"), Bus),
    net("ic9_key", ("ic9", "M_KEY"), ("keypad", "S_ARM9"), Bus),
    net("ic9_ipc", ("ic9", "M_IPC"), ("ipc", "S_ARM9"), Bus),
    net("ic9_gx", ("ic9", "M_GX"), ("ge3d", "S_GXFIFO"), Bus),
    net("ic9_2da", ("ic9", "M_2DA"), ("eng2d_a", "S_REG"), Bus),
    net("ic9_2db", ("ic9", "M_2DB"), ("eng2d_b", "S_REG"), Bus),
    net("ic9_irq", ("ic9", "M_IRQ"), ("irq9", "S_REG"), Bus),
    net("ic9_dma", ("ic9", "M_DMA"), ("dma9", "S_REG"), Bus),
    net("ic9_tmr", ("ic9", "M_TMR"), ("timers9", "S_REG"), Bus),
    net("ic9_math", ("ic9", "M_MATH"), ("math", "S_REG"), Bus),
    // ARM7 bus.
    net("arm7_ahb", ("arm7", "M_AHB"), ("ic7", "S00_CPU"), Bus),
    net("dma7_ahb", ("dma7", "M_AHB"), ("ic7", "S01_DMA"), Bus),
    net("ic7_main", ("ic7", "M_MAIN"), ("main_ram", "S_ARM7"), Bus),
    net("ic7_wram", ("ic7", "M_WRAM"), ("wram", "S_ARM7"), Bus),
    net("ic7_wram7", ("ic7", "M_WRAM7"), ("wram7", "S_ARM7"), Bus),
    net("ic7_vram", ("ic7", "M_VRAM"), ("vram", "S_ARM7"), Bus),
    net("ic7_slot1", ("ic7", "M_SLOT1"), ("slot1", "S_ARM7"), Bus),
    net("ic7_key", ("ic7", "M_KEY"), ("keypad", "S_ARM7"), Bus),
    net("ic7_ipc", ("ic7", "M_IPC"), ("ipc", "S_ARM7"), Bus),
    net("ic7_spu", ("ic7", "M_SPU"), ("spu", "S_REG"), Bus),
    net("ic7_spi", ("ic7", "M_SPI"), ("spi", "S_REG"), Bus),
    net("ic7_rtc", ("ic7", "M_RTC"), ("rtc", "S_REG"), Bus),
    net("ic7_wifi", ("ic7", "M_WIFI"), ("wifi", "S_REG"), Bus),
    net("ic7_irq", ("ic7", "M_IRQ"), ("irq7", "S_REG"), Bus),
    net("ic7_dma", ("ic7", "M_DMA"), ("dma7", "S_REG"), Bus),
    net("ic7_tmr", ("ic7", "M_TMR"), ("timers7", "S_REG"), Bus),
    // CPU interrupt lines.
    net("nirq9", ("irq9", "nIRQ"), ("arm9", "nIRQ"), Signal),
    net("nirq7", ("irq7", "nIRQ"), ("arm7", "nIRQ"), Signal),
    // ARM9 interrupt sources (GBATEK "DS Interrupts").
    irq("irq9_lcd", ("eng2d_a", "VBLANK"), ("irq9", "LCD"), Arm9, 0x0000_0007),
    irq("irq9_timer", ("timers9", "IRQ"), ("irq9", "TIMER"), Arm9, 0x0000_0078),
    irq("irq9_dma", ("dma9", "IRQ"), ("irq9", "DMA"), Arm9, 0x0000_0F00),
    irq("irq9_key", ("keypad", "IRQ"), ("irq9", "KEYPAD"), Arm9, 0x0000_1000),
    irq("irq9_slot2", ("slot2", "IREQ"), ("irq9", "GBA_SLOT"), Arm9, 0x0000_2000),
    irq("irq9_ipc", ("ipc", "IRQ9"), ("irq9", "IPC"), Arm9, 0x0007_0000),
    irq("irq9_card", ("slot1", "IRQ"), ("irq9", "CARD"), Arm9, 0x0018_0000),
    irq("irq9_gx", ("ge3d", "IRQ"), ("irq9", "GXFIFO"), Arm9, 0x0020_0000),
    // ARM7 interrupt sources.
    irq("irq7_timer", ("timers7", "IRQ"), ("irq7", "TIMER"), Arm7, 0x0000_0078),
    irq("irq7_rtc", ("rtc", "IRQ"), ("irq7", "RTC"), Arm7, 0x0000_0080),
    irq("irq7_dma", ("dma7", "IRQ"), ("irq7", "DMA"), Arm7, 0x0000_0F00),
    irq("irq7_ipc", ("ipc", "IRQ7"), ("irq7", "IPC"), Arm7, 0x0007_0000),
    irq("irq7_spi", ("spi", "IRQ"), ("irq7", "SPI"), Arm7, 0x0080_0000),
    irq("irq7_wifi", ("wifi", "IRQ"), ("irq7", "WIFI"), Arm7, 0x0100_0000),
    // Video.
    net("vram_bga", ("vram", "BG_OBJ_A"), ("eng2d_a", "VRAM"), Video),
    net("vram_bgb", ("vram", "BG_OBJ_B"), ("eng2d_b", "VRAM"), Video),
    net("vram_tex", ("vram", "TEX_PAL"), ("re3d", "TEX"), Video),
    net("gx_poly", ("ge3d", "POLY_VTX"), ("re3d", "POLY_VTX"), Video),
    net("re3d_bg0", ("re3d", "PIXELS"), ("eng2d_a", "BG0_3D"), Video),
    net("a_out", ("eng2d_a", "PIXELS"), ("lcd_top", "PIXELS"), Video),
    net("b_out", ("eng2d_b", "PIXELS"), ("lcd_bottom", "PIXELS"), Video),
    // Audio.
    net("spu_out", ("spu", "L_R"), ("speaker", "L_R"), Audio),
    net("mic_in", ("tsc", "MIC"), ("spi", "MIC"), Audio),
    // SPI devices.
    net("spi_pmic", ("spi", "CS0"), ("pmic", "SPI"), Signal),
    net("spi_fw", ("spi", "CS1"), ("firmware", "SPI"), Signal),
    net("spi_tsc", ("spi", "CS2"), ("tsc", "SPI"), Signal),
];

pub fn block_def(id: &str) -> Option<&'static BlockDef> {
    BLOCKS.iter().find(|b| b.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_net_end_is_a_block() {
        for net in NETS {
            assert!(block_def(net.from.0).is_some(), "{}: {}", net.name, net.from.0);
            assert!(block_def(net.to.0).is_some(), "{}: {}", net.name, net.to.0);
        }
    }

    #[test]
    fn ids_and_net_names_are_unique() {
        for (i, b) in BLOCKS.iter().enumerate() {
            assert!(BLOCKS[..i].iter().all(|o| o.id != b.id), "{}", b.id);
        }
        for (i, n) in NETS.iter().enumerate() {
            assert!(NETS[..i].iter().all(|o| o.name != n.name), "{}", n.name);
        }
    }
}
