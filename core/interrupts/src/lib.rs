/// Interrupt identifier for different interrupt sources
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    VBlank,
    HBlank,
    VCountMatch,
    Timer0,
    Timer1,
    Timer2,
    Timer3,
    Rtc,
    Dma0,
    Dma1,
    Dma2,
    Dma3,
    Keypad,
    GBASlot,
    IpcSync = 16,
    IpcFifoEmpty,
    IpcFifoNempty,
    CartTransfer,
    CartIreqMc,
    GeometryFifo,
    UnfoldScreen,
    Spi,
    Wifi,
}

impl Interrupt {
    pub const fn from_usize(value: usize) -> Option<Self> {
        Some(match value {
            0 => Self::VBlank,
            1 => Self::HBlank,
            2 => Self::VCountMatch,
            3 => Self::Timer0,
            4 => Self::Timer1,
            5 => Self::Timer2,
            6 => Self::Timer3,
            7 => Self::Rtc,
            8 => Self::Dma0,
            9 => Self::Dma1,
            10 => Self::Dma2,
            11 => Self::Dma3,
            12 => Self::Keypad,
            13 => Self::GBASlot,
            16 => Self::IpcSync,
            17 => Self::IpcFifoEmpty,
            18 => Self::IpcFifoNempty,
            19 => Self::CartTransfer,
            20 => Self::CartIreqMc,
            21 => Self::GeometryFifo,
            22 => Self::UnfoldScreen,
            23 => Self::Spi,
            24 => Self::Wifi,
            _ => return None,
        })
    }
}

#[derive(Debug, Default)]
pub struct InterruptRegs {
    pub ime: u32,
    /// - `IE`
    pub irq_enable: u32,
    /// - `IF`
    pub irq_flags: u32,
}

impl InterruptRegs {
    pub fn is_requesting_int(&self, _bit: u32) -> bool {
        // (self.irq_enable & bit != 0) && (self.irq_flags & bit != 0)
        unimplemented!("It is not used in C++ and has no definition.");
    }
}
