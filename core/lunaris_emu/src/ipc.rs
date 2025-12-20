/// Inter-Processor Communication (IPC) system for Nintendo DS
/// Enables communication between ARM7 and ARM9 processors
use std::collections::VecDeque;

/// IPC Synchronization register
#[derive(Debug, Clone, Copy)]
pub struct IpcSync {
    /// Input data from other CPU
    pub input: u32,
    /// Output data to other CPU
    pub output: u32,
    /// Enable interrupt on receive
    pub irq_enable: bool,
}

impl IpcSync {
    /// Create new IPC sync register
    pub fn new() -> Self {
        IpcSync {
            input: 0,
            output: 0,
            irq_enable: false,
        }
    }

    /// Read sync register value
    pub fn read(&self) -> u16 {
        let mut value = 0u16;
        value |= ((self.input & 0xF) as u16) << 8;
        value |= ((self.output & 0xF) as u16);
        if self.irq_enable {
            value |= 1 << 14;
        }
        value
    }

    /// Receive input from other CPU
    pub fn receive_input(&mut self, halfword: u16) {
        self.input = ((halfword >> 8) & 0xF) as u32;
    }

    /// Write to sync register
    pub fn write(&mut self, halfword: u16) {
        self.output = (halfword & 0xF) as u32;
        self.irq_enable = (halfword & (1 << 14)) != 0;
    }
}

impl Default for IpcSync {
    fn default() -> Self {
        Self::new()
    }
}

/// IPC FIFO (First-In-First-Out) queue for data transfer
#[derive(Debug, Clone)]
pub struct IpcFifo {
    /// Queue for sending data to other CPU
    pub send_queue: VecDeque<u32>,
    /// Queue for receiving data from other CPU
    pub receive_queue: VecDeque<u32>,

    /// Most recently read data
    pub recent_word: u32,

    /// Send queue is empty IRQ enable
    pub send_empty_irq: bool,
    /// Request to empty send queue
    pub request_empty_irq: bool,
    /// Receive queue not empty IRQ enable
    pub receive_nempty_irq: bool,
    /// Request not empty IRQ
    pub request_nempty_irq: bool,

    /// Error flag (write to full queue or read from empty)
    pub error: bool,
    /// FIFO enabled
    pub enabled: bool,
}

impl IpcFifo {
    /// Create new IPC FIFO
    pub fn new() -> Self {
        IpcFifo {
            send_queue: VecDeque::with_capacity(16),
            receive_queue: VecDeque::with_capacity(16),
            recent_word: 0,
            send_empty_irq: false,
            request_empty_irq: false,
            receive_nempty_irq: false,
            request_nempty_irq: false,
            error: false,
            enabled: false,
        }
    }

    /// Read control register
    pub fn read_cnt(&self) -> u16 {
        let mut value = 0u16;
        if self.send_queue.is_empty() {
            value |= 1 << 0;
        }
        if self.send_queue.len() >= 16 {
            value |= 1 << 1;
        } // Full
        if self.send_empty_irq {
            value |= 1 << 2;
        }
        if self.request_empty_irq {
            value |= 1 << 3;
        }
        if !self.receive_queue.is_empty() {
            value |= 1 << 8;
        }
        if self.receive_queue.len() >= 16 {
            value |= 1 << 9;
        } // Full
        if self.receive_nempty_irq {
            value |= 1 << 10;
        }
        if self.request_nempty_irq {
            value |= 1 << 11;
        }
        if self.error {
            value |= 1 << 14;
        }
        if self.enabled {
            value |= 1 << 15;
        }
        value
    }

    /// Write control register
    pub fn write_cnt(&mut self, value: u16) {
        self.send_empty_irq = (value & (1 << 2)) != 0;
        self.request_empty_irq = (value & (1 << 3)) != 0;
        self.receive_nempty_irq = (value & (1 << 10)) != 0;
        self.request_nempty_irq = (value & (1 << 11)) != 0;
        self.error = (value & (1 << 14)) != 0; // Can be cleared by writing 0
        self.enabled = (value & (1 << 15)) != 0;
    }

    /// Read from receive queue
    pub fn read_queue(&mut self) -> u32 {
        if let Some(word) = self.receive_queue.pop_front() {
            self.recent_word = word;
            word
        } else {
            // Reading from empty queue sets error flag
            self.error = true;
            self.recent_word
        }
    }

    /// Write to send queue
    pub fn write_queue(&mut self, word: u32) {
        if self.send_queue.len() < 16 {
            self.send_queue.push_back(word);
        } else {
            // Writing to full queue sets error flag
            self.error = true;
        }
    }

    /// Check if send queue is empty
    pub fn send_empty(&self) -> bool {
        self.send_queue.is_empty()
    }

    /// Check if send queue is full
    pub fn send_full(&self) -> bool {
        self.send_queue.len() >= 16
    }

    /// Check if receive queue has data
    pub fn receive_not_empty(&self) -> bool {
        !self.receive_queue.is_empty()
    }

    /// Check if receive queue is full
    pub fn receive_full(&self) -> bool {
        self.receive_queue.len() >= 16
    }

    /// Get send queue size
    pub fn send_queue_size(&self) -> usize {
        self.send_queue.len()
    }

    /// Get receive queue size
    pub fn receive_queue_size(&self) -> usize {
        self.receive_queue.len()
    }

    /// Clear error flag
    pub fn clear_error(&mut self) {
        self.error = false;
    }
}

impl Default for IpcFifo {
    fn default() -> Self {
        Self::new()
    }
}

/// Inter-Processor Communication system
/// Manages synchronization and FIFO queues between ARM7 and ARM9
pub struct IPC {
    /// ARM7 sync register
    sync7: IpcSync,
    /// ARM9 sync register
    sync9: IpcSync,

    /// ARM7 FIFO (receives from ARM9, sends to ARM9)
    fifo7: IpcFifo,
    /// ARM9 FIFO (receives from ARM7, sends to ARM7)
    fifo9: IpcFifo,
}

impl IPC {
    /// Create new IPC system
    pub fn new() -> Self {
        IPC {
            sync7: IpcSync::new(),
            sync9: IpcSync::new(),
            fifo7: IpcFifo::new(),
            fifo9: IpcFifo::new(),
        }
    }

    // ARM7 methods

    /// ARM7 read IPCSYNC
    pub fn arm7_read_sync(&self) -> u16 {
        self.sync7.read()
    }

    /// ARM7 write IPCSYNC
    pub fn arm7_write_sync(&mut self, value: u16) {
        self.sync7.write(value);
        // Notify ARM9 of new sync data
        self.sync9.receive_input(value);
    }

    /// ARM7 read from FIFO
    pub fn arm7_read_fifo(&mut self) -> u32 {
        self.fifo7.read_queue()
    }

    /// ARM7 write to FIFO
    pub fn arm7_write_fifo(&mut self, word: u32) {
        self.fifo7.write_queue(word);
        // Add to ARM9's receive queue
        if self.fifo9.receive_queue.len() < 16 {
            self.fifo9.receive_queue.push_back(word);
        } else {
            self.fifo9.error = true;
        }
    }

    /// ARM7 read FIFO control
    pub fn arm7_read_fifo_cnt(&self) -> u16 {
        self.fifo7.read_cnt()
    }

    /// ARM7 write FIFO control
    pub fn arm7_write_fifo_cnt(&mut self, value: u16) {
        self.fifo7.write_cnt(value);
    }

    // ARM9 methods

    /// ARM9 read IPCSYNC
    pub fn arm9_read_sync(&self) -> u16 {
        self.sync9.read()
    }

    /// ARM9 write IPCSYNC
    pub fn arm9_write_sync(&mut self, value: u16) {
        self.sync9.write(value);
        // Notify ARM7 of new sync data
        self.sync7.receive_input(value);
    }

    /// ARM9 read from FIFO
    pub fn arm9_read_fifo(&mut self) -> u32 {
        self.fifo9.read_queue()
    }

    /// ARM9 write to FIFO
    pub fn arm9_write_fifo(&mut self, word: u32) {
        self.fifo9.write_queue(word);
        // Add to ARM7's receive queue
        if self.fifo7.receive_queue.len() < 16 {
            self.fifo7.receive_queue.push_back(word);
        } else {
            self.fifo7.error = true;
        }
    }

    /// ARM9 read FIFO control
    pub fn arm9_read_fifo_cnt(&self) -> u16 {
        self.fifo9.read_cnt()
    }

    /// ARM9 write FIFO control
    pub fn arm9_write_fifo_cnt(&mut self, value: u16) {
        self.fifo9.write_cnt(value);
    }

    // Query methods

    /// Check if ARM7 should receive interrupt (FIFO not empty)
    pub fn arm7_should_irq_receive(&self) -> bool {
        self.fifo7.receive_nempty_irq && self.fifo7.receive_not_empty()
    }

    /// Check if ARM7 should receive interrupt (send queue empty)
    pub fn arm7_should_irq_send_empty(&self) -> bool {
        self.fifo7.send_empty_irq && self.fifo7.send_empty()
    }

    /// Check if ARM9 should receive interrupt (FIFO not empty)
    pub fn arm9_should_irq_receive(&self) -> bool {
        self.fifo9.receive_nempty_irq && self.fifo9.receive_not_empty()
    }

    /// Check if ARM9 should receive interrupt (send queue empty)
    pub fn arm9_should_irq_send_empty(&self) -> bool {
        self.fifo9.send_empty_irq && self.fifo9.send_empty()
    }
}

impl Default for IPC {
    fn default() -> Self {
        Self::new()
    }
}
