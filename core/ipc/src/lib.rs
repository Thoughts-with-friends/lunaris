// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! ipc.hpp
//!
//! Inter-Processor Communication (IPC) system for Nintendo DS
//! Enables communication between ARM7 and ARM9 processors.
//!
//! Modeled after melonDS (`NDS.cpp`: `IPCSync9/7`, `IPCFIFO9/7`,
//! `IPCFIFOCnt9/7`). The original CorgiDS design shared the two FIFO queues
//! between both CPUs' FIFO objects by pointer; here the [`Ipc`] block owns one
//! queue per direction instead, so a word written by one CPU is always
//! visible to the other CPU's reads and FIFOCNT.
//!
//! See GBATEK "DS Inter Process Communication (IPC)".
use std::collections::VecDeque;

/// Hardware FIFO depth (words) per direction.
const FIFO_DEPTH: usize = 16;

/// Which CPU performs an IPC access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Arm9,
    Arm7,
}

impl Side {
    /// The other CPU.
    #[must_use]
    pub const fn remote(self) -> Self {
        match self {
            Self::Arm9 => Self::Arm7,
            Self::Arm7 => Self::Arm9,
        }
    }
}

/// IPC interrupt kinds (IRQ bits 16..=18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcIrq {
    /// IPCSYNC remote request (bit 16)
    Sync,
    /// Send FIFO became empty (bit 17)
    SendEmpty,
    /// Receive FIFO became non-empty (bit 18)
    RecvNotEmpty,
}

/// An IRQ to raise on `target` as a side effect of an IPC access.
pub type IrqRequest = (Side, IpcIrq);

/// IPCFIFOCNT bits stored in the register (the status bits are derived).
const CNT_SEND_EMPTY_IRQ: u16 = 1 << 2;
const CNT_SEND_CLEAR: u16 = 1 << 3;
const CNT_RECV_NEMPTY_IRQ: u16 = 1 << 10;
const CNT_ERROR: u16 = 1 << 14;
const CNT_ENABLE: u16 = 1 << 15;

/// Both CPUs' IPCSYNC/IPCFIFOCNT registers and the two direction queues.
#[derive(Debug, Clone, Default)]
pub struct Ipc {
    /// IPCSYNC as seen by ARM9 (4000180h)
    sync9: u16,
    /// IPCSYNC as seen by ARM7 (4000180h)
    sync7: u16,
    /// IPCFIFOCNT stored bits for ARM9 (4000184h)
    cnt9: u16,
    /// IPCFIFOCNT stored bits for ARM7 (4000184h)
    cnt7: u16,
    /// ARM9 -> ARM7 queue
    fifo9: VecDeque<u32>,
    /// ARM7 -> ARM9 queue
    fifo7: VecDeque<u32>,
    /// Last word popped from `fifo9` (returned on empty / disabled reads)
    last9: u32,
    /// Last word popped from `fifo7`
    last7: u32,
}

impl Ipc {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets all IPC state (power on).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    const fn sync(&self, side: Side) -> u16 {
        match side {
            Side::Arm9 => self.sync9,
            Side::Arm7 => self.sync7,
        }
    }

    const fn sync_mut(&mut self, side: Side) -> &mut u16 {
        match side {
            Side::Arm9 => &mut self.sync9,
            Side::Arm7 => &mut self.sync7,
        }
    }

    const fn cnt(&self, side: Side) -> u16 {
        match side {
            Side::Arm9 => self.cnt9,
            Side::Arm7 => self.cnt7,
        }
    }

    const fn cnt_mut(&mut self, side: Side) -> &mut u16 {
        match side {
            Side::Arm9 => &mut self.cnt9,
            Side::Arm7 => &mut self.cnt7,
        }
    }

    /// Queue that `side` sends into.
    const fn send_queue(&self, side: Side) -> &VecDeque<u32> {
        match side {
            Side::Arm9 => &self.fifo9,
            Side::Arm7 => &self.fifo7,
        }
    }

    /// Reads IPCSYNC.
    ///
    /// Bits 0-3: input from the remote's bits 8-11, bits 8-11: own output,
    /// bit 14: remote IRQ enable.
    #[must_use]
    pub const fn read_sync(&self, side: Side) -> u16 {
        self.sync(side)
    }

    /// Writes IPCSYNC with a byte `mask` (0x00FF, 0xFF00 or 0xFFFF) so that
    /// 8-bit stores to 4000180h/4000181h only touch their own half.
    ///
    /// Returns an IRQ for the remote CPU if bit 13 was written while the
    /// remote has its IPCSYNC IRQ enabled.
    pub const fn write_sync(&mut self, side: Side, value: u16, mask: u16) -> Option<IrqRequest> {
        let remote = side.remote();
        if mask & 0xFF00 != 0 {
            let out = (value >> 8) & 0xF;
            let own = self.sync_mut(side);
            *own = (*own & !0x4F00) | (value & 0x4F00);
            let other = self.sync_mut(remote);
            *other = (*other & !0x000F) | out;

            if value & (1 << 13) != 0 && self.sync(remote) & (1 << 14) != 0 {
                return Some((remote, IpcIrq::Sync));
            }
        }
        None
    }

    /// Reads IPCFIFOCNT.
    #[must_use]
    pub fn read_cnt(&self, side: Side) -> u16 {
        let send = self.send_queue(side);
        let recv = self.send_queue(side.remote());
        let mut value = self.cnt(side);
        if send.is_empty() {
            value |= 1 << 0;
        } else if send.len() >= FIFO_DEPTH {
            value |= 1 << 1;
        }
        if recv.is_empty() {
            value |= 1 << 8;
        } else if recv.len() >= FIFO_DEPTH {
            value |= 1 << 9;
        }
        value
    }

    /// Writes IPCFIFOCNT (melonDS `ARM9IOWrite16` 0x04000184).
    ///
    /// Enabling an IRQ whose condition already holds raises it immediately.
    pub fn write_cnt(&mut self, side: Side, value: u16) -> [Option<IrqRequest>; 2] {
        let old = self.cnt(side);
        if value & CNT_SEND_CLEAR != 0 {
            match side {
                Side::Arm9 => self.fifo9.clear(),
                Side::Arm7 => self.fifo7.clear(),
            }
        }

        let mut irqs = [None, None];
        if value & CNT_SEND_EMPTY_IRQ != 0
            && old & CNT_SEND_EMPTY_IRQ == 0
            && self.send_queue(side).is_empty()
        {
            irqs[0] = Some((side, IpcIrq::SendEmpty));
        }
        if value & CNT_RECV_NEMPTY_IRQ != 0
            && old & CNT_RECV_NEMPTY_IRQ == 0
            && !self.send_queue(side.remote()).is_empty()
        {
            irqs[1] = Some((side, IpcIrq::RecvNotEmpty));
        }

        let error = if value & CNT_ERROR != 0 {
            0 // write-1-to-acknowledge
        } else {
            old & CNT_ERROR
        };

        *self.cnt_mut(side) =
            (value & (CNT_ENABLE | CNT_RECV_NEMPTY_IRQ | CNT_SEND_EMPTY_IRQ)) | error;
        irqs
    }

    /// Writes IPCFIFOSEND (4000188h).
    ///
    /// Raises the remote's receive-not-empty IRQ when the queue goes from
    /// empty to non-empty.
    pub fn send(&mut self, side: Side, word: u32) -> Option<IrqRequest> {
        if self.cnt(side) & CNT_ENABLE == 0 {
            return None;
        }
        let remote = side.remote();
        let queue = match side {
            Side::Arm9 => &mut self.fifo9,
            Side::Arm7 => &mut self.fifo7,
        };
        if queue.len() >= FIFO_DEPTH {
            *self.cnt_mut(side) |= CNT_ERROR;
            return None;
        }
        let was_empty = queue.is_empty();
        queue.push_back(word);
        if was_empty && self.cnt(remote) & CNT_RECV_NEMPTY_IRQ != 0 {
            return Some((remote, IpcIrq::RecvNotEmpty));
        }
        None
    }

    /// Reads IPCFIFORECV (4100000h).
    ///
    /// Raises the remote's send-empty IRQ when its queue drains.
    pub fn receive(&mut self, side: Side) -> (u32, Option<IrqRequest>) {
        let remote = side.remote();
        let enabled = self.cnt(side) & CNT_ENABLE != 0;
        let (queue, last) = match remote {
            Side::Arm9 => (&mut self.fifo9, &mut self.last9),
            Side::Arm7 => (&mut self.fifo7, &mut self.last7),
        };
        if !enabled {
            return (queue.front().copied().unwrap_or(*last), None);
        }
        if let Some(word) = queue.pop_front() {
            *last = word;
            let drained = queue.is_empty();
            let irq = (drained && self.cnt(remote) & CNT_SEND_EMPTY_IRQ != 0)
                .then_some((remote, IpcIrq::SendEmpty));
            (word, irq)
        } else {
            let word = *last;
            *self.cnt_mut(side) |= CNT_ERROR;
            (word, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_output_reaches_remote_input() {
        let mut ipc = Ipc::new();
        // ARM9 byte-writes 0x5 to 4000181h
        assert_eq!(ipc.write_sync(Side::Arm9, 0x0500, 0xFF00), None);
        assert_eq!(ipc.read_sync(Side::Arm7) & 0xF, 0x5);
        assert_eq!(ipc.read_sync(Side::Arm9), 0x0500);
        // byte write to 4000180h is a no-op
        ipc.write_sync(Side::Arm9, 0x0F, 0x00FF);
        assert_eq!(ipc.read_sync(Side::Arm9), 0x0500);
    }

    #[test]
    fn sync_irq_requires_remote_enable() {
        let mut ipc = Ipc::new();
        assert_eq!(ipc.write_sync(Side::Arm9, 1 << 13, 0xFFFF), None);
        ipc.write_sync(Side::Arm7, 1 << 14, 0xFFFF);
        assert_eq!(
            ipc.write_sync(Side::Arm9, 1 << 13, 0xFFFF),
            Some((Side::Arm7, IpcIrq::Sync))
        );
    }

    #[test]
    fn fifo_crosses_cpus() {
        let mut ipc = Ipc::new();
        ipc.write_cnt(Side::Arm9, CNT_ENABLE);
        ipc.write_cnt(Side::Arm7, CNT_ENABLE | CNT_RECV_NEMPTY_IRQ);
        assert_eq!(ipc.read_cnt(Side::Arm7) & (1 << 8), 1 << 8);
        assert_eq!(
            ipc.send(Side::Arm9, 0xDEAD_BEEF),
            Some((Side::Arm7, IpcIrq::RecvNotEmpty))
        );
        assert_eq!(ipc.read_cnt(Side::Arm9) & 1, 0); // ARM9 send not empty
        assert_eq!(ipc.read_cnt(Side::Arm7) & (1 << 8), 0); // ARM7 recv not empty
        assert_eq!(ipc.receive(Side::Arm7).0, 0xDEAD_BEEF);
        assert_eq!(ipc.read_cnt(Side::Arm9) & 1, 1);
        // empty read sets the error flag and returns the last word
        assert_eq!(ipc.receive(Side::Arm7).0, 0xDEAD_BEEF);
        assert_ne!(ipc.read_cnt(Side::Arm7) & CNT_ERROR, 0);
    }
}
