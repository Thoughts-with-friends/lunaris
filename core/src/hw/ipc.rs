//! NDS Inter-Processor Communication (IPC).
//!
//! The NDS provides two IPC mechanisms:
//!
//! 1. **IPCSYNC** (4000180h) – 4-bit data register each way; writing to the
//!    output field of one CPU sets the input field of the other.  Can optionally
//!    fire an `IPC_SYNC` IRQ on the receiving CPU.
//!
//! 2. **IPCFIFO** (IPCFIFOCNT 4000184h / IPCFIFOSEND 4000188h / IPCFIFORECV
//!    4100000h) – 16-word (64-byte) first-in-first-out queues, one per
//!    direction.  Each CPU writes to its own send-FIFO; the other CPU reads
//!    from the same queue via IPCFIFORECV.  Fire IRQs on empty/not-empty
//!    transitions when the corresponding enable bits are set.
//!
//! GBATEK "DS Inter Process Communication (IPC)":
//! <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>

use std::collections::VecDeque;

use super::interrupt_controller::InterruptRequest;

/// Shared IPC state visible to both CPUs.
///
/// Each CPU has its own [`SYNC`] and [`FIFOCNT`] registers, and a send-FIFO
/// (`output7` / `output9`) that the *other* CPU reads from.
/// `prev_value*` retains the last word dequeued so a read from an empty
/// FIFO returns a defined value (last received word).
///
/// GBATEK "4100000h - IPCFIFORECV" (empty-FIFO read behaviour):
/// <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>
#[derive(emu_utils::Savestate)]
pub struct IPC {
    fifocnt7: FIFOCNT,
    sync7: SYNC,
    /// ARM7 send-FIFO (ARM9 reads this via IPCFIFORECV at 4100000h).
    ///
    /// `VecDeque::load_in_place` does not consume the stored length prefix,
    /// so route through `Loadable` instead.
    /// See `docs/design/savestate-and-video-design.md`.
    #[load(with = "save.load()?", with_in_place = "*output7 = save.load()?")]
    output7: VecDeque<u32>,
    prev_value7: u32,
    fifocnt9: FIFOCNT,
    sync9: SYNC,
    /// ARM9 send-FIFO (ARM7 reads this via IPCFIFORECV at 4100000h).
    #[load(with = "save.load()?", with_in_place = "*output9 = save.load()?")]
    output9: VecDeque<u32>,
    prev_value9: u32,
}

impl IPC {
    /// Maximum words per FIFO queue (16 words / 64 bytes each direction).
    ///
    /// GBATEK: <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>
    const FIFO_LEN: usize = 16;

    pub fn new() -> Self {
        IPC {
            fifocnt7: FIFOCNT::new(),
            sync7: SYNC::new(),
            output7: VecDeque::new(),
            prev_value7: 0,
            fifocnt9: FIFOCNT::new(),
            sync9: SYNC::new(),
            output9: VecDeque::new(),
            prev_value9: 0,
        }
    }

    pub fn read_sync7(&self, byte: usize) -> u8 {
        self.sync7.read(byte)
    }
    pub fn read_sync9(&self, byte: usize) -> u8 {
        self.sync9.read(byte)
    }
    pub fn read_fifocnt7(&self, byte: usize) -> u8 {
        self.fifocnt7.read(&self.output7, &self.output9, byte)
    }
    pub fn read_fifocnt9(&self, byte: usize) -> u8 {
        self.fifocnt9.read(&self.output9, &self.output7, byte)
    }
    pub fn arm7_recv(&mut self) -> (u32, InterruptRequest) {
        IPC::recv(&self.fifocnt9, &mut self.fifocnt7, &mut self.output9, &mut self.prev_value9)
    }
    pub fn arm9_recv(&mut self) -> (u32, InterruptRequest) {
        IPC::recv(&self.fifocnt7, &mut self.fifocnt9, &mut self.output7, &mut self.prev_value7)
    }

    pub fn write_sync7(&mut self, byte: usize, value: u8) -> InterruptRequest {
        self.sync7.write(&mut self.sync9, byte, value)
    }
    pub fn write_sync9(&mut self, byte: usize, value: u8) -> InterruptRequest {
        self.sync9.write(&mut self.sync7, byte, value)
    }
    pub fn write_fifocnt7(&mut self, byte: usize, value: u8) -> InterruptRequest {
        let prev_fifocnt = self.fifocnt7;
        self.fifocnt7.write(&mut self.output7, &mut self.prev_value7, byte, value);
        IPC::check_fifo_interrupt(&self.output7, &self.output9, &prev_fifocnt, &self.fifocnt7)
    }
    pub fn write_fifocnt9(&mut self, byte: usize, value: u8) -> InterruptRequest {
        let prev_fifocnt = self.fifocnt9;
        self.fifocnt9.write(&mut self.output9, &mut self.prev_value9, byte, value);
        IPC::check_fifo_interrupt(&self.output9, &self.output7, &prev_fifocnt, &self.fifocnt9)
    }
    pub fn arm7_send(&mut self, value: u32) -> InterruptRequest {
        IPC::send(&mut self.fifocnt7, &self.fifocnt9, &mut self.output7, value)
    }
    pub fn arm9_send(&mut self, value: u32) -> InterruptRequest {
        IPC::send(&mut self.fifocnt9, &self.fifocnt7, &mut self.output9, value)
    }

    fn check_fifo_interrupt(
        send_fifo: &VecDeque<u32>,
        recv_fifo: &VecDeque<u32>,
        prev_cnt: &FIFOCNT,
        new_cnt: &FIFOCNT,
    ) -> InterruptRequest {
        let empty_condition =
            send_fifo.is_empty() && !prev_cnt.send_fifo_empty_irq && new_cnt.send_fifo_empty_irq;
        let not_empty_condition = !recv_fifo.is_empty()
            && !prev_cnt.recv_fifo_not_empty_irq
            && new_cnt.recv_fifo_not_empty_irq;

        (if empty_condition {
            InterruptRequest::IPC_SEND_FIFO_EMPTY
        } else {
            InterruptRequest::empty()
        }) | (if not_empty_condition {
            InterruptRequest::IPC_RECV_FIFO_NOT_EMPTY
        } else {
            InterruptRequest::empty()
        })
    }

    /// Pops one word from `recv_fifo`.
    ///
    /// - Returns the cached `prev_value` when FIFO is empty (hardware behaviour:
    ///   "last received word").  Sets `FIFOCNT.error` on underflow instead of
    ///   returning garbage.  GBATEK "IPCFIFOCNT Bit 14 Error flag":
    ///   <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>
    /// - Fires `IPC_SEND_FIFO_EMPTY` on the sender when the FIFO drains and the
    ///   sender's empty-IRQ enable is set.
    fn recv(
        send_cnt: &FIFOCNT,
        recv_cnt: &mut FIFOCNT,
        recv_fifo: &mut VecDeque<u32>,
        prev_value: &mut u32,
    ) -> (u32, InterruptRequest) {
        if !recv_cnt.enable {
            return (*prev_value, InterruptRequest::empty());
        }
        assert!(send_cnt.enable); // TODO: Figure out behavior
        let interrupt = if let Some(value) = recv_fifo.pop_front() {
            *prev_value = value;
            if send_cnt.enable && send_cnt.send_fifo_empty_irq && recv_fifo.is_empty() {
                InterruptRequest::IPC_SEND_FIFO_EMPTY
            } else {
                InterruptRequest::empty()
            }
        } else {
            recv_cnt.error = true;
            InterruptRequest::empty()
        };
        (*prev_value, interrupt)
    }

    /// Pushes one word onto `send_fifo`.
    ///
    /// Sets `FIFOCNT.error` on overflow (FIFO already has 16 words).
    /// Fires `IPC_RECV_FIFO_NOT_EMPTY` on the receiver when the FIFO goes from
    /// empty to non-empty and the receiver's not-empty IRQ enable is set.
    /// GBATEK "IPCFIFOSEND / error flag on full FIFO":
    /// <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>
    fn send(
        send_cnt: &mut FIFOCNT,
        recv_cnt: &FIFOCNT,
        send_fifo: &mut VecDeque<u32>,
        value: u32,
    ) -> InterruptRequest {
        if !send_cnt.enable {
            return InterruptRequest::empty();
        }
        let interrupt =
            if recv_cnt.enable && recv_cnt.recv_fifo_not_empty_irq && send_fifo.is_empty() {
                InterruptRequest::IPC_RECV_FIFO_NOT_EMPTY
            } else {
                InterruptRequest::empty()
            };
        if send_fifo.len() == IPC::FIFO_LEN {
            send_cnt.error = true;
        } else {
            send_fifo.push_back(value);
        }
        interrupt
    }
}

/// IPCSYNC register (4000180h).
///
/// Bit layout (16-bit view):
/// - Bits 0-3 (byte 0): Data received from other CPU (read-only, set by other side's write).
/// - Bits 8-11 (byte 1, bits 0-3): Data to send to other CPU (read-write).
/// - Bit 13 (byte 1, bit 5): Send IRQ to other CPU (pulse; triggers IPC_SYNC on other side).
/// - Bit 14 (byte 1, bit 6): Enable IPC_SYNC IRQ on *this* CPU.
///
/// GBATEK "4000180h - IPCSYNC":
/// <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>
#[derive(emu_utils::Savestate)]
struct SYNC {
    /// Bits [3:0] mirrored from the other CPU's `output`.
    input: u8,
    /// Bits [11:8] written by this CPU; mirrored into the other CPU's `input`.
    output: u8,
    /// Whether to fire IPC_SYNC IRQ on this CPU when the other side sets bit 13.
    sync_irq: bool,
}

impl SYNC {
    fn new() -> Self {
        SYNC { input: 0, output: 0, sync_irq: false }
    }

    fn read(&self, byte: usize) -> u8 {
        match byte {
            0 => self.input,
            1 => (self.sync_irq as u8) << 6 | self.output,
            2 => 0,
            3 => 0,
            _ => unreachable!(),
        }
    }

    fn write(&mut self, other: &mut Self, byte: usize, value: u8) -> InterruptRequest {
        if match byte {
            0 => false,
            1 => {
                self.output = value;
                other.input = self.output;
                self.sync_irq = value >> 6 & 0x1 != 0;
                other.sync_irq && value >> 5 & 0x1 != 0
            }
            2 => false,
            3 => false,
            _ => unreachable!(),
        } {
            InterruptRequest::IPC_SYNC
        } else {
            InterruptRequest::empty()
        }
    }
}

/// IPCFIFOCNT register (4000184h).
///
/// Byte 0 (send side):
/// - Bit 0: Send-FIFO empty flag (read-only).
/// - Bit 1: Send-FIFO full flag (read-only).
/// - Bit 2: `send_fifo_empty_irq` – fire IPC_SEND_FIFO_EMPTY when FIFO becomes empty.
/// - Bit 3: Flush send-FIFO (write-only; also clears `prev_value`).
///
/// Byte 1 (recv side):
/// - Bit 0: Recv-FIFO empty flag (read-only).
/// - Bit 1: Recv-FIFO full flag (read-only).
/// - Bit 2: `recv_fifo_not_empty_irq` – fire IPC_RECV_FIFO_NOT_EMPTY.
/// - Bit 6: Error flag (read-only; write 1 to acknowledge/clear).
/// - Bit 7: `enable` – enable FIFO mode (both FIFOs at once).
///
/// GBATEK "4000184h - IPCFIFOCNT":
/// <https://problemkaputt.de/gbatek.htm#dsinterprocesscommunicationipc>
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
struct FIFOCNT {
    send_fifo_empty_irq: bool,
    recv_fifo_not_empty_irq: bool,
    /// Set on FIFO overflow or underflow; cleared by writing 1 to bit 6.
    error: bool,
    enable: bool,
}

impl FIFOCNT {
    fn new() -> Self {
        FIFOCNT {
            send_fifo_empty_irq: false,
            recv_fifo_not_empty_irq: false,
            error: false,
            enable: false,
        }
    }

    fn read(&self, send_fifo: &VecDeque<u32>, recv_fifo: &VecDeque<u32>, byte: usize) -> u8 {
        match byte {
            0 => (self.send_fifo_empty_irq as u8) << 2 | FIFOCNT::get_fifo_status(send_fifo),
            1 => {
                (self.enable as u8) << 7
                    | (self.error as u8) << 6
                    | (self.recv_fifo_not_empty_irq as u8) << 2
                    | FIFOCNT::get_fifo_status(recv_fifo)
            }
            2 => 0,
            3 => 0,
            _ => unreachable!(),
        }
    }

    fn write(
        &mut self,
        send_fifo: &mut VecDeque<u32>,
        prev_output: &mut u32,
        byte: usize,
        value: u8,
    ) {
        match byte {
            0 => {
                self.send_fifo_empty_irq = value >> 2 & 0x1 != 0;
                if value >> 3 & 0x1 != 0 {
                    send_fifo.clear();
                    *prev_output = 0;
                }
            }
            1 => {
                self.recv_fifo_not_empty_irq = value >> 2 & 0x1 != 0;
                self.error = self.error && (value >> 6) & 0x1 == 0; // 1 means acknowledge error
                self.enable = value >> 7 & 0x1 != 0;
            }
            2 => (),
            3 => (),
            _ => unreachable!(),
        }
    }

    fn get_fifo_status(fifo: &VecDeque<u32>) -> u8 {
        assert!(fifo.len() <= IPC::FIFO_LEN);
        ((fifo.len() == IPC::FIFO_LEN) as u8) << 1 | fifo.is_empty() as u8
    }
}
