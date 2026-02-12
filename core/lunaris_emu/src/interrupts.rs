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
    pub fn from_usize(value: usize) -> Option<Self> {
        Some(match value {
            0 => Interrupt::VBlank,
            1 => Interrupt::HBlank,
            2 => Interrupt::VCountMatch,
            3 => Interrupt::Timer0,
            4 => Interrupt::Timer1,
            5 => Interrupt::Timer2,
            6 => Interrupt::Timer3,
            7 => Interrupt::Rtc,
            8 => Interrupt::Dma0,
            9 => Interrupt::Dma1,
            10 => Interrupt::Dma2,
            11 => Interrupt::Dma3,
            12 => Interrupt::Keypad,
            13 => Interrupt::GBASlot,
            16 => Interrupt::IpcSync,
            17 => Interrupt::IpcFifoEmpty,
            18 => Interrupt::IpcFifoNempty,
            19 => Interrupt::CartTransfer,
            20 => Interrupt::CartIreqMc,
            21 => Interrupt::GeometryFifo,
            22 => Interrupt::UnfoldScreen,
            23 => Interrupt::Spi,
            24 => Interrupt::Wifi,
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
    #[allow(unused)]
    pub fn is_requesting_int(&self, bit: u32) -> bool {
        // (self.irq_enable & bit != 0) && (self.irq_flags & bit != 0)
        unimplemented!("It is not used in C++ and has no definition.");
    }
}
