//! TX slot management: starting a transmission, the preamble/transmit/
//! MP-reply-window phase machine, and handing finished frames to the
//! [`super::mp::MpTransport`]. Ported from melonDS
//! `docs/design/melonds/WiFi.cpp:601-1110` (`TXSendFrame`, `StartTX_*`,
//! `ProcessTX`), simplified to the six-slot subset needed for MP mode.
//!
//! Slot map (mirrors hardware `W_TXBusy` bit assignment):
//!
//! | Slot | `W_TXBusy` bit | Address register  | Role                     |
//! |------|----------------|--------------------|--------------------------|
//! | 0    | `0x0001`       | `W_TXSlotLoc1`     | general frame (LOC1)     |
//! | 1    | `0x0002`       | `W_TXSlotCmd`      | host MP command          |
//! | 2    | `0x0004`       | `W_TXSlotLoc2`     | general frame (LOC2)     |
//! | 3    | `0x0008`       | `W_TXSlotLoc3`     | general frame (LOC3)     |
//! | 4    | `0x0010`       | `W_TXSlotBeacon`   | beacon                   |
//! | 5    | `0x0080`       | `W_TXSlotReply1`   | client MP reply (auto)   |

use super::Wifi;
use super::regs::*;
use crate::hw::interrupt_controller::InterruptRequest;

const SLOT_BUSY_BITS: [u16; 6] = [0x0001, 0x0002, 0x0004, 0x0008, 0x0010, 0x0080];
const SLOT_ADDR_REG: [usize; 6] =
    [W_TXSlotLoc1, W_TXSlotCmd, W_TXSlotLoc2, W_TXSlotLoc3, W_TXSlotBeacon, W_TXSlotReply1];

fn preamble_len(rate: u8) -> i32 {
    if rate == 2 { 96 } else { 192 }
}

impl Wifi {
    /// Handles a `W_TXReqSet` write: for each newly-set bit, latches the
    /// slot's frame address/length/rate from WiFi RAM and (if idle) begins
    /// its preamble phase.
    pub(super) fn try_start_tx(&mut self, requested: u16) {
        for (slot, &bit) in SLOT_BUSY_BITS.iter().enumerate() {
            if slot == 5 {
                continue; // Reply slot is hardware-triggered, not CPU-triggered.
            }
            if requested & bit == 0 {
                continue;
            }
            let addr_reg = self.ioport(SLOT_ADDR_REG[slot]);
            let addr = (addr_reg & 0x0FFF) << 1;
            let length = self.ram_u16(addr as usize + 0xA) & 0x3FFF;
            let rate_byte = self.ram[addr as usize + 0x8];
            let rate = if rate_byte == 0x14 { 2 } else { 1 };

            self.tx_slots[slot].valid = true;
            self.tx_slots[slot].addr = addr;
            self.tx_slots[slot].length = length;
            self.tx_slots[slot].rate = rate;
            self.tx_slots[slot].phase = 0;
            self.tx_slots[slot].phase_time = preamble_len(rate);

            if slot == 1 {
                // Host CMD: latch the client mask this command targets,
                // clearing any clients already marked failed.
                let mask = self.ram_u16(addr as usize + 12 + 24 + 2) & self.mp_client_fail;
                self.mp_client_mask = mask;
                self.mp_client_fail &= mask;
            }
        }
    }

    /// Starts the automatic MP reply slot (5) in response to a host CMD
    /// frame addressed to us. Mirrors `SendMPReply`
    /// (`docs/design/melonds/WiFi.cpp:800-826`), simplified: replies carry
    /// no payload beyond the 12-byte hardware header plus a fixed-size
    /// placeholder body, which is sufficient for MP handshake/keepalive
    /// traffic but not for games that inspect large reply payloads.
    pub(super) fn start_mp_reply(&mut self, rate: u8) {
        if self.tx_slots[5].valid {
            return;
        }
        self.tx_slots[5].valid = true;
        self.tx_slots[5].addr = 0; // Reply frames are synthesized directly into tx_buffer.
        self.tx_slots[5].length = 28;
        self.tx_slots[5].rate = rate;
        self.tx_slots[5].phase = 0;
        self.tx_slots[5].phase_time = preamble_len(rate);
        self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) | 0x0080);
    }

    fn ram_u16(&self, addr: usize) -> u16 {
        if addr + 1 >= self.ram.len() {
            return 0;
        }
        self.ram[addr] as u16 | (self.ram[addr + 1] as u16) << 8
    }

    /// Advances the highest-priority busy TX slot by one 8µs timer tick.
    /// Returns `true` once no slot remains busy.
    pub(super) fn process_tx(&mut self, request: &mut InterruptRequest) {
        let Some(slot) = self.pick_busy_slot() else {
            self.com_status &= !0x2;
            return;
        };

        self.tx_slots[slot].phase_time -= super::Wifi::TIMER_INTERVAL_US as i32;
        if self.tx_slots[slot].phase_time > 0 {
            return;
        }

        match self.tx_slots[slot].phase {
            0 => {
                // Preamble complete: send the frame body now.
                self.raise_irq(7, request);
                if self.cur_channel != 0 {
                    self.send_slot_frame(slot);
                }
                self.tx_slots[slot].phase = 1;
                self.tx_slots[slot].phase_time = self.tx_slots[slot].length as i32
                    * if self.tx_slots[slot].rate == 2 { 4 } else { 8 };
            }
            1 => {
                // Transmit complete.
                self.finish_slot(slot, request);
            }
            2 => {
                // MP reply-collection window elapsed (host, slot 1 only):
                // broadcast the acknowledgement, carrying the current
                // adaptive run-ahead window, then close the slot.
                self.send_mp_ack();
                self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !(1 << 1));
                self.raise_irq(1, request);
                self.tx_slots[1].valid = false;
                self.com_status &= !0x2;
            }
            _ => {}
        }
    }

    fn pick_busy_slot(&self) -> Option<usize> {
        let busy = self.ioport(W_TXBusy);
        SLOT_BUSY_BITS.iter().enumerate().rev().find(|&(_, &bit)| busy & bit != 0).map(|(i, _)| i)
    }

    fn finish_slot(&mut self, slot: usize, request: &mut InterruptRequest) {
        match slot {
            1 => {
                // Host CMD sent: open the reply-collection window and poll
                // the transport for any replies already queued.
                self.mp_reply_timer = 16 + preamble_len(self.tx_slots[1].rate);
                if self.mp_client_mask != 0
                    && let Some(mut transport) = self.transport.take()
                {
                    let mut buf = vec![0u8; self.mp_client_replies.len()];
                    let answered =
                        transport.recv_replies(&mut buf, self.us_timestamp, self.mp_client_mask);
                    self.mp_client_replies[..buf.len()].copy_from_slice(&buf);
                    self.mp_client_fail &= !answered;
                    self.transport = Some(transport);
                }
                self.tx_slots[1].phase = 2;
                let num_clients = self.mp_client_mask.count_ones();
                self.tx_slots[1].phase_time =
                    112 + (10 + self.ioport(W_CmdReplyTime) as i32) * num_clients as i32;
            }
            5 => {
                self.raise_irq(1, request);
                self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !0x0080);
                self.tx_slots[5].valid = false;
                self.com_status &= !0x2;
            }
            _ => {
                self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !SLOT_BUSY_BITS[slot]);
                self.raise_irq(1, request);
                self.tx_slots[slot].valid = false;
                self.com_status &= !0x2;
            }
        }
    }

    /// Builds the 12-byte hardware header + frame body into `tx_buffer` and
    /// hands it to the transport. See `docs/design/design_lan.md` §5.5-§5.6.
    fn send_slot_frame(&mut self, slot: usize) {
        let s = self.tx_slots[slot];
        if slot == 5 {
            self.send_mp_reply(s.rate);
            return;
        }

        let len = (s.length as usize).min(Wifi::TX_BUFFER_SIZE.saturating_sub(12));
        let src_end = (s.addr as usize + 12 + len).min(self.ram.len());
        let src_start = (s.addr as usize).min(src_end);
        let copy_len = src_end - src_start;
        self.tx_buffer[..copy_len].copy_from_slice(&self.ram[src_start..src_end]);
        self.tx_buffer[9] = self.cur_channel as u8;
        self.tx_buffer[8] = if s.rate == 2 { 0x14 } else { 0x0A };
        let frame_len_bytes = copy_len.saturating_sub(12) as u16;
        self.tx_buffer[10] = frame_len_bytes as u8;
        self.tx_buffer[11] = (frame_len_bytes >> 8) as u8;

        let Some(mut transport) = self.transport.take() else { return };
        match slot {
            1 => {
                transport.send_cmd(&self.tx_buffer[..copy_len], self.us_timestamp);
            }
            _ => {
                transport.send_packet(&self.tx_buffer[..copy_len], self.us_timestamp);
            }
        }
        self.transport = Some(transport);
    }

    fn send_mp_reply(&mut self, rate: u8) {
        let aid = self.ioport(W_AIDLow);
        let hints = self.link_hints();
        self.tx_buffer[0] = 0x01;
        self.tx_buffer[1] = 0x00;
        self.tx_buffer[8] = if rate == 2 { 0x14 } else { 0x0A };
        self.tx_buffer[9] = self.cur_channel as u8;
        self.tx_buffer[10] = 28;
        self.tx_buffer[11] = 0;
        let Some(mut transport) = self.transport.take() else { return };
        transport.send_reply(&self.tx_buffer[..40], self.us_timestamp, aid);
        let _ = hints;
        self.transport = Some(transport);
    }

    /// Sends a host acknowledgement frame carrying the run-ahead window
    /// clients may consume before their next mandatory sync point.
    pub(super) fn send_mp_ack(&mut self) {
        let hints = self.link_hints();
        self.tx_buffer[0] = 0x01;
        self.tx_buffer[9] = self.cur_channel as u8;
        self.tx_buffer[10] = 32;
        self.tx_buffer[11] = 0;
        let Some(mut transport) = self.transport.take() else { return };
        transport.send_ack(&self.tx_buffer[..44], self.us_timestamp, hints.runahead_us);
        self.transport = Some(transport);
    }
}
