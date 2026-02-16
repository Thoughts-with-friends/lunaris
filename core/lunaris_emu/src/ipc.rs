// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! ipc.hpp
//!
//! Inter-Processor Communication (IPC) system for Nintendo DS
//! Enables communication between ARM7 and ARM9 processors
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
        value |= (self.output & 0xF) as u16;
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
    ///
    /// - NOTE: ptr
    pub send_queue: VecDeque<u32>,
    /// Queue for receiving data from other CPU
    /// - NOTE: ptr
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
        if !self.enabled {
            return self.recent_word;
        }

        match self.send_queue.front() {
            Some(&word) => {
                self.send_queue.pop_back();
                if self.send_queue.is_empty() && self.send_empty_irq {
                    self.request_empty_irq = true;
                }

                word
            }
            None => {
                self.error = true;
                self.recent_word
            }
        }
    }

    /// Write to send queue
    pub fn write_queue(&mut self, word: u32) {
        if !self.enabled {
            return;
        }

        if self.receive_queue.len() >= 16 {
            self.error = true;
        } else {
            if self.receive_queue.is_empty() && self.request_nempty_irq {
                self.request_nempty_irq = true;
            }
            self.receive_queue.push_back(word);
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
