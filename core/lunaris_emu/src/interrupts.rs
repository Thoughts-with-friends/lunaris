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

#[derive(Debug, Default)]
pub struct InterruptRegs {
    pub ime: u32,
    /// - `IE`
    pub irq_enable: u32,
    /// - `IF`
    pub irq_flags: u32,
}

impl InterruptRegs {
    fn is_requesting_int(bit: i32) -> bool {
        todo!()
    }
}
