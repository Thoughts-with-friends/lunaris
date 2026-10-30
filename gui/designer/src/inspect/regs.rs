//! Live registers of every block: I/O address, name, width, a reader and
//! the bit fields to decode it with (GBATEK names and layouts).
//!
//! Readers look at core state directly (getters and fields) rather than
//! going through the CPU bus, so inspecting never has side effects: no
//! FIFO is popped, no cartridge word consumed, nothing is logged.

use std::collections::HashMap;

use ds_emu::Emulator;
use ds_peripherals::ipc::Side;

/// Bits `lo..=hi` of a register.
pub struct Field {
    pub lo: u8,
    pub hi: u8,
    pub name: &'static str,
}

impl Field {
    pub const fn extract(&self, value: u32) -> u32 {
        let width = self.hi - self.lo + 1;
        let mask = if width >= 32 { u32::MAX } else { (1 << width) - 1 };
        (value >> self.lo) & mask
    }
}

pub type Reader = Box<dyn Fn(&Emulator) -> u32>;

pub struct Reg {
    /// I/O address; 0 for core-internal state (CPU registers...).
    pub addr: u32,
    pub name: String,
    /// 8, 16 or 32.
    pub bits: u8,
    pub read: Reader,
    pub fields: &'static [Field],
}

const fn f(lo: u8, hi: u8, name: &'static str) -> Field {
    Field { lo, hi, name }
}

fn reg(
    addr: u32,
    name: impl Into<String>,
    bits: u8,
    read: impl Fn(&Emulator) -> u32 + 'static,
    fields: &'static [Field],
) -> Reg {
    Reg { addr, name: name.into(), bits, read: Box::new(read), fields }
}

const DISPCNT: &[Field] = &[
    f(0, 2, "BG mode"),
    f(3, 3, "BG0 is 3D"),
    f(4, 4, "tile OBJ 1D"),
    f(7, 7, "forced blank"),
    f(8, 8, "BG0 on"),
    f(9, 9, "BG1 on"),
    f(10, 10, "BG2 on"),
    f(11, 11, "BG3 on"),
    f(12, 12, "OBJ on"),
    f(13, 13, "WIN0"),
    f(14, 14, "WIN1"),
    f(15, 15, "OBJ window"),
    f(16, 17, "display mode"),
    f(18, 19, "LCDC VRAM block"),
    f(20, 21, "tile OBJ boundary"),
    f(22, 22, "bitmap OBJ boundary"),
    f(23, 23, "OBJ in HBlank"),
    f(24, 26, "char base (64K)"),
    f(27, 29, "screen base (64K)"),
    f(30, 30, "BG ext palettes"),
    f(31, 31, "OBJ ext palettes"),
];
const BGCNT: &[Field] = &[
    f(0, 1, "priority"),
    f(2, 5, "char base (16K)"),
    f(6, 6, "mosaic"),
    f(7, 7, "256 colours"),
    f(8, 12, "screen base (2K)"),
    f(13, 13, "ext pal slot / wrap"),
    f(14, 15, "size"),
];
const POWCNT1: &[Field] = &[
    f(0, 0, "LCDs"),
    f(1, 1, "2D A"),
    f(2, 2, "3D render"),
    f(3, 3, "3D geometry"),
    f(9, 9, "2D B"),
    f(15, 15, "A on top LCD"),
];
const VRAMCNT: &[Field] = &[f(0, 2, "MST"), f(3, 4, "offset"), f(7, 7, "enable")];
const IRQ9: &[Field] = &[
    f(0, 0, "VBlank"),
    f(1, 1, "HBlank"),
    f(2, 2, "VCount"),
    f(3, 3, "TM0"),
    f(4, 4, "TM1"),
    f(5, 5, "TM2"),
    f(6, 6, "TM3"),
    f(8, 8, "DMA0"),
    f(9, 9, "DMA1"),
    f(10, 10, "DMA2"),
    f(11, 11, "DMA3"),
    f(12, 12, "keypad"),
    f(13, 13, "GBA slot"),
    f(16, 16, "IPC sync"),
    f(17, 17, "IPC send empty"),
    f(18, 18, "IPC recv not empty"),
    f(19, 19, "card done"),
    f(20, 20, "card IREQ"),
    f(21, 21, "GX FIFO"),
];
const IRQ7: &[Field] = &[
    f(0, 0, "VBlank"),
    f(1, 1, "HBlank"),
    f(2, 2, "VCount"),
    f(3, 3, "TM0"),
    f(4, 4, "TM1"),
    f(5, 5, "TM2"),
    f(6, 6, "TM3"),
    f(7, 7, "RTC / SIO"),
    f(8, 8, "DMA0"),
    f(9, 9, "DMA1"),
    f(10, 10, "DMA2"),
    f(11, 11, "DMA3"),
    f(12, 12, "keypad"),
    f(13, 13, "GBA slot"),
    f(16, 16, "IPC sync"),
    f(17, 17, "IPC send empty"),
    f(18, 18, "IPC recv not empty"),
    f(19, 19, "card done"),
    f(20, 20, "card IREQ"),
    f(22, 22, "screens unfolding"),
    f(23, 23, "SPI"),
    f(24, 24, "Wi-Fi"),
];
const EXMEMCNT: &[Field] = &[
    f(0, 1, "Slot-2 SRAM wait"),
    f(7, 7, "Slot-2 to ARM7"),
    f(11, 11, "Slot-1 to ARM7"),
    f(15, 15, "main RAM priority"),
];
const DMACNT: &[Field] = &[
    f(5, 6, "dest ctrl"),
    f(7, 8, "src ctrl"),
    f(9, 9, "repeat"),
    f(10, 10, "32-bit"),
    f(11, 13, "start mode"),
    f(14, 14, "IRQ"),
    f(15, 15, "enable"),
];
const TMCNT: &[Field] =
    &[f(0, 1, "prescaler"), f(2, 2, "count-up"), f(6, 6, "IRQ"), f(7, 7, "start")];
const IPCSYNC: &[Field] =
    &[f(0, 3, "data in"), f(8, 11, "data out"), f(13, 13, "send IRQ"), f(14, 14, "IRQ enable")];
const IPCFIFOCNT: &[Field] = &[
    f(0, 0, "send empty"),
    f(1, 1, "send full"),
    f(2, 2, "send empty IRQ"),
    f(8, 8, "recv empty"),
    f(9, 9, "recv full"),
    f(10, 10, "recv IRQ"),
    f(14, 14, "error"),
    f(15, 15, "enable"),
];
const SOUNDCNT: &[Field] = &[
    f(0, 6, "master volume"),
    f(8, 9, "left output"),
    f(10, 11, "right output"),
    f(12, 12, "ch1 not mixed"),
    f(13, 13, "ch3 not mixed"),
    f(15, 15, "enable"),
];
const SOUNDXCNT: &[Field] = &[
    f(0, 6, "volume"),
    f(8, 9, "divider"),
    f(15, 15, "hold"),
    f(16, 22, "panning"),
    f(24, 26, "PSG duty"),
    f(27, 28, "repeat"),
    f(29, 30, "format"),
    f(31, 31, "busy"),
];
const KEYINPUT: &[Field] = &[
    f(0, 0, "A (0=down)"),
    f(1, 1, "B"),
    f(2, 2, "Select"),
    f(3, 3, "Start"),
    f(4, 4, "Right"),
    f(5, 5, "Left"),
    f(6, 6, "Up"),
    f(7, 7, "Down"),
    f(8, 8, "R"),
    f(9, 9, "L"),
];
const EXTKEYIN: &[Field] = &[
    f(0, 0, "X (0=down)"),
    f(1, 1, "Y"),
    f(3, 3, "debug"),
    f(6, 6, "pen up"),
    f(7, 7, "hinge closed"),
];
const SPICNT: &[Field] = &[
    f(0, 1, "baudrate"),
    f(7, 7, "busy"),
    f(8, 9, "device"),
    f(10, 10, "16-bit"),
    f(11, 11, "hold CS"),
    f(14, 14, "IRQ"),
    f(15, 15, "enable"),
];
const AUXSPICNT: &[Field] = &[
    f(0, 1, "baudrate"),
    f(6, 6, "hold CS"),
    f(7, 7, "busy"),
    f(13, 13, "SPI mode"),
    f(14, 14, "IRQ"),
    f(15, 15, "enable"),
];
const ROMCTRL: &[Field] = &[
    f(0, 12, "KEY1 gap1"),
    f(13, 13, "KEY2 data"),
    f(22, 22, "KEY2 cmd"),
    f(23, 23, "data ready"),
    f(24, 26, "block size"),
    f(27, 27, "clock"),
    f(29, 29, "release reset"),
    f(30, 30, "write"),
    f(31, 31, "busy"),
];
const GXSTAT: &[Field] = &[
    f(0, 0, "test busy"),
    f(1, 1, "box test result"),
    f(8, 12, "pos/vec stack"),
    f(13, 13, "proj stack"),
    f(14, 14, "stack busy"),
    f(15, 15, "stack error"),
    f(16, 24, "FIFO entries"),
    f(25, 25, "FIFO full"),
    f(26, 26, "less than half"),
    f(27, 27, "FIFO empty"),
    f(28, 28, "busy"),
    f(30, 31, "FIFO IRQ"),
];
const DISP3DCNT: &[Field] = &[
    f(0, 0, "textures"),
    f(1, 1, "highlight"),
    f(2, 2, "alpha test"),
    f(3, 3, "alpha blend"),
    f(4, 4, "anti-alias"),
    f(5, 5, "edge marking"),
    f(6, 6, "fog alpha only"),
    f(7, 7, "fog"),
    f(8, 11, "fog shift"),
    f(14, 14, "rear bitmap"),
];
const MASTER_BRIGHT: &[Field] = &[f(0, 4, "factor"), f(14, 15, "mode")];
const DISPSTAT: &[Field] = &[
    f(0, 0, "VBlank"),
    f(1, 1, "HBlank"),
    f(2, 2, "VCount match"),
    f(3, 3, "VBlank IRQ"),
    f(4, 4, "HBlank IRQ"),
    f(5, 5, "VCount IRQ"),
    f(7, 15, "VCount setting"),
];
const POWCNT2: &[Field] = &[f(0, 0, "speakers"), f(1, 1, "Wi-Fi")];
const DIVCNT: &[Field] = &[f(0, 1, "mode"), f(14, 14, "divide by zero"), f(15, 15, "busy")];
const CPSR: &[Field] = &[
    f(0, 4, "mode"),
    f(5, 5, "Thumb"),
    f(6, 6, "FIQ off"),
    f(7, 7, "IRQ off"),
    f(28, 28, "V"),
    f(29, 29, "C"),
    f(30, 30, "Z"),
    f(31, 31, "N"),
];
const NONE: &[Field] = &[];

fn cpu_regs(arm9: bool) -> Vec<Reg> {
    let mut regs: Vec<Reg> = (0..16)
        .map(|i| {
            let name = match i {
                13 => "SP".to_owned(),
                14 => "LR".to_owned(),
                15 => "PC".to_owned(),
                _ => format!("R{i}"),
            };
            reg(0, name, 32, move |e| (if arm9 { &e.arm9 } else { &e.arm7 }).regs[i], NONE)
        })
        .collect();
    regs.push(reg(0, "CPSR", 32, move |e| (if arm9 { &e.arm9 } else { &e.arm7 }).cpsr.get(), CPSR));
    regs.push(reg(
        0,
        "HALTED",
        8,
        move |e| u32::from((if arm9 { &e.arm9 } else { &e.arm7 }).halted),
        NONE,
    ));
    regs
}

fn engine_2d(b: bool) -> Vec<Reg> {
    let base = if b { 0x0400_1000 } else { 0x0400_0000 };
    let mut regs = vec![reg(
        base,
        "DISPCNT",
        32,
        move |e| if b { e.gpu.get_dispcnt_b() } else { e.gpu.get_dispcnt_a() },
        DISPCNT,
    )];
    for i in 0..4 {
        regs.push(reg(
            base + 8 + 2 * i as u32,
            format!("BG{i}CNT"),
            16,
            move |e| u32::from(if b { e.gpu.get_bgcnt_b(i) } else { e.gpu.get_bgcnt_a(i) }),
            BGCNT,
        ));
    }
    for i in 0..4 {
        regs.push(reg(
            base + 0x10 + 4 * i as u32,
            format!("BG{i}HOFS"),
            16,
            move |e| u32::from(if b { e.gpu.get_bghofs_b(i) } else { e.gpu.get_bghofs_a(i) }),
            NONE,
        ));
        regs.push(reg(
            base + 0x12 + 4 * i as u32,
            format!("BG{i}VOFS"),
            16,
            move |e| u32::from(if b { e.gpu.get_bgvofs_b(i) } else { e.gpu.get_bgvofs_a(i) }),
            NONE,
        ));
    }
    regs.push(reg(
        base + 0x48,
        "WININ",
        16,
        move |e| u32::from(if b { e.gpu.get_winin_b() } else { e.gpu.get_winin_a() }),
        NONE,
    ));
    regs.push(reg(
        base + 0x4A,
        "WINOUT",
        16,
        move |e| u32::from(if b { e.gpu.get_winout_b() } else { e.gpu.get_winout_a() }),
        NONE,
    ));
    regs.push(reg(
        base + 0x50,
        "BLDCNT",
        16,
        move |e| u32::from(if b { e.gpu.get_bldcnt_b() } else { e.gpu.get_bldcnt_a() }),
        NONE,
    ));
    regs.push(reg(
        base + 0x52,
        "BLDALPHA",
        16,
        move |e| u32::from(if b { e.gpu.get_bldalpha_b() } else { e.gpu.get_bldalpha_a() }),
        NONE,
    ));
    regs.push(reg(
        base + 0x6C,
        "MASTER_BRIGHT",
        16,
        move |e| {
            u32::from(if b { e.gpu.get_master_bright_b() } else { e.gpu.get_master_bright_a() })
        },
        MASTER_BRIGHT,
    ));
    if !b {
        regs.push(reg(0x0400_0064, "DISPCAPCNT", 32, |e| e.gpu.get_dispcapcnt_a(), NONE));
    }
    regs
}

fn dma(arm7: bool) -> Vec<Reg> {
    let mut regs = Vec::new();
    for ch in 0..4 {
        let index = ch + if arm7 { 4 } else { 0 };
        let base = 0x0400_00B0 + 12 * ch as u32;
        regs.push(reg(base, format!("DMA{ch}SAD"), 32, move |e| e.dma.read_source(index), NONE));
        regs.push(reg(
            base + 8,
            format!("DMA{ch}CNT_L"),
            16,
            move |e| u32::from(e.dma.read_len(index)),
            NONE,
        ));
        regs.push(reg(
            base + 10,
            format!("DMA{ch}CNT_H"),
            16,
            move |e| u32::from(e.dma.read_cnt(index)),
            DMACNT,
        ));
    }
    regs
}

fn timers(arm7: bool) -> Vec<Reg> {
    let mut regs = Vec::new();
    for t in 0..4 {
        let index = t + if arm7 { 0 } else { 4 };
        let base = 0x0400_0100 + 4 * t as u32;
        regs.push(reg(
            base,
            format!("TM{t}CNT_L"),
            16,
            move |e| u32::from(e.nds_timing.read_lo(index)),
            NONE,
        ));
        regs.push(reg(
            base + 2,
            format!("TM{t}CNT_H"),
            16,
            move |e| u32::from(e.nds_timing.read_hi(index)),
            TMCNT,
        ));
    }
    regs
}

fn irq(arm7: bool) -> Vec<Reg> {
    let fields = if arm7 { IRQ7 } else { IRQ9 };
    vec![
        reg(
            0x0400_0208,
            "IME",
            32,
            move |e| (if arm7 { &e.int7_reg } else { &e.int9_reg }).ime,
            NONE,
        ),
        reg(
            0x0400_0210,
            "IE",
            32,
            move |e| (if arm7 { &e.int7_reg } else { &e.int9_reg }).irq_enable,
            fields,
        ),
        reg(
            0x0400_0214,
            "IF",
            32,
            move |e| (if arm7 { &e.int7_reg } else { &e.int9_reg }).irq_flags,
            fields,
        ),
    ]
}

/// A cheap fingerprint of a memory, so the design can show it is in use.
fn sample(mem: &[u8]) -> u32 {
    mem.chunks(4096)
        .fold(0u32, |acc, c| acc.rotate_left(5) ^ u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
}

fn fpga_accesses(e: &Emulator) -> u32 {
    e.slot2
        .device()
        .and_then(|d| d.as_any().downcast_ref::<ds_fpga::FpgaSlot>())
        .map_or(0, |f| f.accesses as u32)
}

/// Registers of every block, built once.
pub struct Catalog {
    map: HashMap<&'static str, Vec<Reg>>,
}

impl Catalog {
    pub fn new() -> Self {
        let vramcnt = |bank: usize| {
            let name = format!("VRAMCNT_{}", (b'A' + bank as u8) as char);
            let addr = 0x0400_0240 + bank as u32 + u32::from(bank >= 7);
            reg(
                addr,
                name,
                8,
                move |e| {
                    u32::from(match bank {
                        0 => e.gpu.get_vramcnt_a(),
                        1 => e.gpu.get_vramcnt_b(),
                        2 => e.gpu.get_vramcnt_c(),
                        3 => e.gpu.get_vramcnt_d(),
                        4 => e.gpu.get_vramcnt_e(),
                        5 => e.gpu.get_vramcnt_f(),
                        6 => e.gpu.get_vramcnt_g(),
                        7 => e.gpu.get_vramcnt_h(),
                        _ => e.gpu.get_vramcnt_i(),
                    })
                },
                VRAMCNT,
            )
        };
        let lcd = || {
            vec![
                reg(0x0400_0004, "DISPSTAT", 16, |e| u32::from(e.gpu.get_dispstat9()), DISPSTAT),
                reg(0x0400_0006, "VCOUNT", 16, |e| u32::from(e.gpu.get_vcount()), NONE),
                reg(0x0400_0304, "POWCNT1", 16, |e| u32::from(e.gpu.get_powcnt1()), POWCNT1),
            ]
        };
        let spu_channels = (0..16).map(|ch| {
            let addr = 0x0400_0400 + 16 * ch;
            reg(addr, format!("SOUND{ch}CNT"), 32, move |e| e.spu.read32(addr), SOUNDXCNT)
        });

        let mut map: HashMap<&'static str, Vec<Reg>> = HashMap::new();
        map.insert("arm9", cpu_regs(true));
        map.insert("arm7", cpu_regs(false));
        map.insert("irq9", irq(false));
        map.insert("irq7", irq(true));
        map.insert("dma9", dma(false));
        map.insert("dma7", dma(true));
        map.insert("timers9", timers(false));
        map.insert("timers7", timers(true));
        map.insert(
            "ic9",
            vec![
                reg(0x0400_0204, "EXMEMCNT", 16, |e| u32::from(e.ex_mem_cnt), EXMEMCNT),
                reg(0x0400_0300, "POSTFLG", 8, |e| u32::from(e.postflg9), NONE),
            ],
        );
        map.insert("ic7", vec![reg(0x0400_0300, "POSTFLG", 8, |e| u32::from(e.postflg7), NONE)]);
        map.insert(
            "math",
            vec![
                reg(0x0400_0280, "DIVCNT", 16, |e| u32::from(e.divcnt), DIVCNT),
                reg(0x0400_0290, "DIV_NUMER_L", 32, |e| e.div_numer as u32, NONE),
                reg(0x0400_0294, "DIV_NUMER_H", 32, |e| (e.div_numer >> 32) as u32, NONE),
                reg(0x0400_0298, "DIV_DENOM_L", 32, |e| e.div_denom as u32, NONE),
                reg(0x0400_029C, "DIV_DENOM_H", 32, |e| (e.div_denom >> 32) as u32, NONE),
                reg(0x0400_02A0, "DIV_RESULT_L", 32, |e| e.div_result as u32, NONE),
                reg(0x0400_02A8, "DIVREM_RESULT_L", 32, |e| e.div_remresult as u32, NONE),
                reg(0x0400_02B0, "SQRTCNT", 16, |e| u32::from(e.sqrtcnt), NONE),
                reg(0x0400_02B4, "SQRT_RESULT", 32, |e| e.sqrt_result, NONE),
                reg(0x0400_02B8, "SQRT_PARAM_L", 32, |e| e.sqrt_param as u32, NONE),
            ],
        );
        map.insert(
            "ipc",
            vec![
                reg(
                    0x0400_0180,
                    "IPCSYNC (ARM9)",
                    16,
                    |e| u32::from(e.ipc.read_sync(Side::Arm9)),
                    IPCSYNC,
                ),
                reg(
                    0x0400_0184,
                    "IPCFIFOCNT (ARM9)",
                    16,
                    |e| u32::from(e.ipc.read_cnt(Side::Arm9)),
                    IPCFIFOCNT,
                ),
                reg(
                    0x0400_0180,
                    "IPCSYNC (ARM7)",
                    16,
                    |e| u32::from(e.ipc.read_sync(Side::Arm7)),
                    IPCSYNC,
                ),
                reg(
                    0x0400_0184,
                    "IPCFIFOCNT (ARM7)",
                    16,
                    |e| u32::from(e.ipc.read_cnt(Side::Arm7)),
                    IPCFIFOCNT,
                ),
            ],
        );
        map.insert(
            "main_ram",
            vec![reg(0x0200_0000, "content sample", 32, |e| sample(&e.main_ram), NONE)],
        );
        map.insert(
            "wram",
            vec![
                reg(0x0400_0247, "WRAMCNT", 8, |e| u32::from(e.wram_cnt), &[]),
                reg(0x0300_0000, "content sample", 32, |e| sample(&e.shared_wram), NONE),
            ],
        );
        map.insert(
            "wram7",
            vec![reg(0x0380_0000, "content sample", 32, |e| sample(&e.arm7_wram), NONE)],
        );
        let mut vram: Vec<Reg> = (0..9).map(vramcnt).collect();
        vram.push(reg(
            0x0400_0240,
            "VRAMSTAT (ARM7)",
            8,
            |e| u32::from(e.gpu.get_vramstat()),
            NONE,
        ));
        map.insert("vram", vram);
        map.insert(
            "slot1",
            vec![
                reg(0x0400_01A0, "AUXSPICNT", 16, |e| u32::from(e.cart.get_auxspicnt()), AUXSPICNT),
                reg(0x0400_01A2, "AUXSPIDATA", 8, |e| u32::from(e.cart.read_auxspidata()), NONE),
                reg(0x0400_01A4, "ROMCTRL", 32, |e| e.cart.get_romctrl(), ROMCTRL),
            ],
        );
        map.insert(
            "slot2",
            vec![
                reg(0x0400_0204, "EXMEMCNT", 16, |e| u32::from(e.ex_mem_cnt), EXMEMCNT),
                reg(0, "FPGA accesses", 32, fpga_accesses, NONE),
            ],
        );
        map.insert(
            "keypad",
            vec![
                reg(0x0400_0130, "KEYINPUT", 16, |e| u32::from(e.key_input.get_value()), KEYINPUT),
                reg(
                    0x0400_0136,
                    "EXTKEYIN (ARM7)",
                    16,
                    |e| u32::from(e.ext_key_in.get_value()),
                    EXTKEYIN,
                ),
            ],
        );
        let mut spu = vec![
            reg(0x0400_0500, "SOUNDCNT", 16, |e| e.spu.read32(0x0400_0500), SOUNDCNT),
            reg(0x0400_0504, "SOUNDBIAS", 16, |e| e.spu.read32(0x0400_0504), NONE),
            reg(0x0400_0508, "SNDCAPCNT", 16, |e| e.spu.read32(0x0400_0508), NONE),
        ];
        spu.extend(spu_channels);
        map.insert("spu", spu);
        map.insert(
            "speaker",
            vec![
                reg(0x0400_0500, "SOUNDCNT", 16, |e| e.spu.read32(0x0400_0500), SOUNDCNT),
                reg(
                    0x0400_0304,
                    "POWCNT2 (ARM7)",
                    16,
                    |e| u32::from(e.pow_cnt2.get_value()),
                    POWCNT2,
                ),
            ],
        );
        map.insert(
            "spi",
            vec![
                reg(0x0400_01C0, "SPICNT", 16, |e| u32::from(e.spi.get_spicnt()), SPICNT),
                reg(0x0400_01C2, "SPIDATA", 8, |e| u32::from(e.spi.read_spidata()), NONE),
            ],
        );
        map.insert(
            "pmic",
            vec![reg(0x0400_0304, "POWCNT2", 16, |e| u32::from(e.pow_cnt2.get_value()), POWCNT2)],
        );
        map.insert(
            "tsc",
            vec![reg(
                0x0400_0136,
                "EXTKEYIN",
                16,
                |e| u32::from(e.ext_key_in.get_value()),
                EXTKEYIN,
            )],
        );
        map.insert("rtc", vec![reg(0x0400_0138, "RTC", 16, |e| u32::from(e.rtc.read()), NONE)]);
        map.insert("wifi", vec![reg(0, "W clock (us, low)", 32, |e| e.wifi.clock() as u32, NONE)]);
        map.insert(
            "ge3d",
            vec![
                reg(0x0400_0600, "GXSTAT", 32, |e| e.gpu.engine_3d.get_gxstat(), GXSTAT),
                reg(0x0400_0604, "RAM_COUNT", 32, |e| e.gpu.engine_3d.read_reg(0x0400_0604), NONE),
            ],
        );
        map.insert(
            "re3d",
            vec![reg(
                0x0400_0060,
                "DISP3DCNT",
                16,
                |e| u32::from(e.gpu.get_disp3dcnt()),
                DISP3DCNT,
            )],
        );
        map.insert("eng2d_a", engine_2d(false));
        map.insert("eng2d_b", engine_2d(true));
        map.insert("lcd_top", lcd());
        map.insert("lcd_bottom", lcd());
        Self { map }
    }

    pub fn registers(&self, block: &str) -> &[Reg] {
        self.map.get(block).map_or(&[], Vec::as_slice)
    }
}
