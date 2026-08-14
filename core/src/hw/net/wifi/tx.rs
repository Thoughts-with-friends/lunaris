//! TX slot management: starting a transmission, the preamble/transmit/
//! MP-reply-window phase machine, and handing finished frames to the
//! [`super::mp::MpTransport`]. Ported from melonDS
//! `docs/design/melonds/WiFi.cpp:601-1203` (`TXSendFrame`, `StartTX_*`,
//! `FireTX`, `ProcessTX`), simplified to the six-slot subset needed for MP
//! mode. See `docs/design/local-mp-melonds-parity.md` for the gap-by-gap
//! rationale behind this file.
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

use super::{Wifi, regs::*};
use crate::hw::interrupt_controller::InterruptRequest;

const SLOT_BUSY_BITS: [u16; 6] = [0x0001, 0x0002, 0x0004, 0x0008, 0x0010, 0x0080];
const SLOT_ADDR_REG: [usize; 6] =
    [W_TXSlotLoc1, W_TXSlotCmd, W_TXSlotLoc2, W_TXSlotLoc3, W_TXSlotBeacon, W_TXSlotReply1];

/// Counts the client bits (association IDs `1..=15`) set in `mask`.
/// Association ID `0` is the host itself and is never a client; the naive
/// `count_ones()` over all 16 bits inflates every MP timing window by one.
/// Ported from `NumClients` (`Wifi.cpp:577-585`).
fn num_clients(mask: u16) -> u32 {
    (1..16).filter(|i| mask & (1 << i) != 0).count() as u32
}

/// Highest-priority busy slot, in melonDS's `USTimer` order
/// (5 → 4 → 3 → 2 → 1 → 0).
///
/// Called only on the idle → transmitting transition and after a slot
/// reports finished -- **never** mid-transmission. Re-scanning every tick
/// (the previous behaviour) let a higher-index slot preempt one already in
/// flight; see [`Wifi::process_tx`] and
/// `docs/design/local-mp-melonds-parity-2.md` F3.
pub(super) fn pick_busy_slot(busy: u16) -> Option<usize> {
    SLOT_BUSY_BITS.iter().enumerate().rev().find(|&(_, &bit)| busy & bit != 0).map(|(i, _)| i)
}

impl Wifi {
    /// Preamble duration in 8µs ticks. Long (1 Mbit) transmissions always
    /// use the long preamble; short (2 Mbit) transmissions use the short
    /// preamble only if the driver opted in via `W_Preamble` bit 2. Ported
    /// from `PreambleLen` (`Wifi.cpp:570-575`).
    fn preamble_len(&self, rate: u8) -> i32 {
        if rate == 1 {
            192
        } else if self.ioport(W_Preamble) & 0x0004 != 0 {
            96
        } else {
            192
        }
    }

    /// Reads a slot's address/length/rate triple out of Wi-Fi RAM at
    /// `addr_reg`'s pointed-to location.
    fn read_slot_frame_info(&self, addr_reg: usize) -> (u16, u16, u8) {
        let addr = (self.ioport(addr_reg) & 0x0FFF) << 1;
        let length = self.ram_u16(addr as usize + 0xA) & 0x3FFF;
        let rate_byte = self.ram.get(addr as usize + 0x8).copied().unwrap_or(0);
        let rate = if rate_byte == 0x14 { 2 } else { 1 };
        (addr, length, rate)
    }

    /// Starts a general-purpose (LOC1-3) transmit slot. Ported from
    /// `StartTX_LocN` (`Wifi.cpp:680-698`); `nslot` doubles as the index
    /// into [`SLOT_ADDR_REG`] because for slots 0/2/3 that register is
    /// exactly `W_TXSlotLoc1 + loc*4`.
    fn start_tx_locn(&mut self, nslot: usize) {
        let (addr, length, rate) = self.read_slot_frame_info(SLOT_ADDR_REG[nslot]);
        self.tx_slots[nslot].valid = true;
        self.tx_slots[nslot].addr = addr;
        self.tx_slots[nslot].length = length;
        self.tx_slots[nslot].rate = rate;
        self.tx_slots[nslot].phase = 0;
        self.tx_slots[nslot].phase_time = self.preamble_len(rate);
    }

    /// Starts the host MP command slot (1). Ported from `StartTX_Cmd`
    /// (`Wifi.cpp:700-736`), including the `CmdCounter` timeout check that
    /// starts the slot straight into the failure phase (13) when there
    /// isn't enough time left for a full CMD/reply/ack round. See
    /// `docs/design/local-mp-melonds-parity.md` Gap 1.4.
    ///
    /// Ends with `UpdatePowerStatus(1)` to force-wake the transceiver, as
    /// melonDS does (`Wifi.cpp:735`): the host must be awake to run the
    /// command round it just armed.
    fn start_tx_cmd(&mut self) {
        let (addr, length, rate) = self.read_slot_frame_info(W_TXSlotCmd);

        // Latch the client mask this command addresses, clearing any
        // clients already marked failed from a previous round.
        let mask = self.ram_u16(addr as usize + 12 + 24 + 2) & self.mp_client_fail;
        self.mp_client_mask = mask;
        self.mp_client_fail &= mask;

        self.tx_slots[1].valid = true;
        self.tx_slots[1].addr = addr;
        self.tx_slots[1].length = length;
        self.tx_slots[1].rate = rate;

        let rate_factor = if rate == 2 { 4 } else { 8 };
        let mut duration = self.preamble_len(rate) + i32::from(length) * rate_factor;
        duration += 112 + (10 + i32::from(self.ioport(W_CmdReplyTime))) * num_clients(mask) as i32;
        duration += 32 * rate_factor;

        if self.cmd_counter as i32 > duration + 100 {
            self.tx_slots[1].phase = 0;
            self.tx_slots[1].phase_time = self.preamble_len(rate);
        } else {
            self.tx_slots[1].phase = 13;
            self.tx_slots[1].phase_time = self.cmd_counter as i32 - 100;
        }

        self.update_power_status(1);
    }

    /// Starts the beacon slot (4). Ported from `StartTX_Beacon`
    /// (`Wifi.cpp:738-755`); called from [`Wifi::ms_timer`]'s beacon-interval
    /// handling, not from [`Wifi::fire_tx`] -- the beacon slot is
    /// IRQ14-triggered, not CPU-request-triggered.
    pub(super) fn start_tx_beacon(&mut self) {
        self.diag.beacon_tx += 1;
        let (addr, length, rate) = self.read_slot_frame_info(W_TXSlotBeacon);
        self.tx_slots[4].valid = true;
        self.tx_slots[4].addr = addr;
        self.tx_slots[4].length = length;
        self.tx_slots[4].rate = rate;
        self.tx_slots[4].phase = 0;
        self.tx_slots[4].phase_time = self.preamble_len(rate);
        self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) | 0x0010);
    }

    /// Starts (at most) one highest-priority requested TX slot. Ported from
    /// `FireTX` (`Wifi.cpp:757-800`).
    ///
    /// This is the register-write-driven counterpart to `W_TXReqSet`/slot
    /// register writes/`W_RXCnt` bit 15; see
    /// `docs/design/local-mp-melonds-parity.md` Gap 1.2. Also seeds
    /// [`Wifi::mp_client_fail`] to `0xFFFE` right before starting a CMD
    /// slot (Gap 1.1) -- without this, `mp_client_mask` is always `0` and
    /// the host never expects a single reply.
    pub(super) fn fire_tx(&mut self) {
        self.diag.fire_tx_calls += 1;
        if self.ioport(W_RXCnt) & 0x8000 == 0 {
            self.diag.fire_tx_rx_disabled += 1;
            return;
        }

        let txbusy = self.ioport(W_TXBusy);
        let txreq = self.ioport(W_TXReqRead);
        let mut txstart = 0u16;
        if self.ioport(W_TXSlotLoc1) & 0x8000 != 0 {
            txstart |= 0x0001;
        }
        if self.ioport(W_TXSlotCmd) & 0x8000 != 0 {
            txstart |= 0x0002;
        }
        if self.ioport(W_TXSlotLoc2) & 0x8000 != 0 {
            txstart |= 0x0004;
        }
        if self.ioport(W_TXSlotLoc3) & 0x8000 != 0 {
            txstart |= 0x0008;
        }
        txstart &= txreq;
        txstart &= !txbusy;

        self.set_ioport(W_TXBusy, txbusy | txstart);

        if super::debug_enabled() && txstart != 0 {
            eprintln!("[wifi] fire_tx starting slot bits=0x{txstart:04X}");
        }

        if txstart & 0x0008 != 0 {
            self.start_tx_locn(3);
        } else if txstart & 0x0004 != 0 {
            self.start_tx_locn(2);
        } else if txstart & 0x0002 != 0 {
            self.diag.cmd_tx += 1;
            self.mp_client_fail = 0xFFFE;
            self.start_tx_cmd();
        } else if txstart & 0x0001 != 0 {
            self.start_tx_locn(0);
        }
    }

    /// Starts the automatic MP reply slot (5) in response to a host CMD
    /// frame addressed to us. Ported from `SendMPReply`
    /// (`Wifi.cpp:835-894`): hands `W_TXSlotReply1` off to `W_TXSlotReply2`,
    /// and only actually transmits the game's staged reply frame if it
    /// fits the host's reply-time budget; otherwise falls back to
    /// [`Wifi::send_mp_default_reply`]. See
    /// `docs/design/local-mp-melonds-parity.md` Gap 3.1.
    pub(super) fn start_mp_reply(&mut self, clienttime: u16, clientmask: u16) {
        if super::debug_enabled() {
            eprintln!("[wifi] start_mp_reply us_timestamp={}", self.us_timestamp);
        }
        if self.ioport(W_TXSlotReply2) & 0x8000 != 0 {
            // Mark the previous reply as sent successfully.
            let prev_addr = (self.ioport(W_TXSlotReply2) & 0x0FFF) << 1;
            self.set_ram_u16(prev_addr as usize, 0x0001);
        }

        self.set_ioport(W_TXSlotReply2, self.ioport(W_TXSlotReply1));
        self.set_ioport(W_TXSlotReply1, 0);

        let rate = 2u8;
        let valid_frame = self.ioport(W_TXSlotReply2) & 0x8000 != 0;
        let (addr, length) = if valid_frame {
            let addr = (self.ioport(W_TXSlotReply2) & 0x0FFF) << 1;
            let length = self.ram_u16(addr as usize + 0xA) & 0x3FFF;
            (addr, length)
        } else {
            (0, 0)
        };

        let rate_factor = if rate == 2 { 4 } else { 8 };
        let fits_budget = valid_frame
            && (self.preamble_len(rate) + i32::from(length) * rate_factor) as u16 <= clienttime;

        self.tx_slots[5].rate = rate;
        let clientnum = num_clients(clientmask & ((1 << self.ioport(W_AIDLow)) - 1));

        // melonDS transmits the reply frame *synchronously* here, inside
        // `SendMPReply` itself -- unlike every other slot, which transmits
        // later at its own preamble-done tick
        // (`ProcessTX`'s `case 0: if (num != 5) TXSendFrame(...)`, i.e.
        // explicitly skipped for the reply slot). The phase/`phase_time`
        // set below exist purely to track simulated completion timing for
        // IRQ/status bookkeeping, not to gate when the data actually goes
        // out. Missing this was Gap 3.1's root cause: the reply was armed
        // but never actually reached the transport.
        if fits_budget {
            self.tx_slots[5].valid = true;
            self.tx_slots[5].addr = addr;
            self.tx_slots[5].length = length;
            self.tx_slots[5].phase = 0;
            self.send_mp_reply(rate);
        } else {
            self.tx_slots[5].valid = true;
            self.tx_slots[5].addr = 0;
            self.tx_slots[5].length = 0;
            self.tx_slots[5].phase = 10;
            self.send_mp_default_reply();
        }
        self.tx_slots[5].phase_time =
            16 + (i32::from(clienttime) + 10) * clientnum as i32 + self.preamble_len(rate);

        self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) | 0x0080);
    }

    fn ram_u16(&self, addr: usize) -> u16 {
        if addr + 1 >= self.ram.len() {
            return 0;
        }
        self.ram[addr] as u16 | (self.ram[addr + 1] as u16) << 8
    }

    fn set_ram_u16(&mut self, addr: usize, value: u16) {
        if addr + 1 >= self.ram.len() {
            return;
        }
        self.ram[addr] = value as u8;
        self.ram[addr + 1] = (value >> 8) as u8;
    }

    /// Increments the per-client MP reply-failure counter for every client bit
    /// set in `clientfail`.
    ///
    /// These are **byte**-wide counters packed two per 16-bit port starting at
    /// [`W_CMDStat0`], which melonDS addresses as a flat byte array via
    /// `IOPORT8(W_CMDStat0 + i)`: client `i` lives in port
    /// `W_CMDStat0 + (i & !1)`, low byte for even `i` and high byte for odd `i`.
    /// A host game reads them to decide whether a client has gone
    /// unresponsive. Ported from `ReportMPReplyErrors` (`Wifi.cpp:594-605`);
    /// see `docs/design/local-mp-melonds-parity-2.md` F6.
    pub(super) fn report_mp_reply_errors(&mut self, clientfail: u16) {
        for i in 1..16usize {
            if clientfail & (1 << i) == 0 {
                continue;
            }
            let port = W_CMDStat0 + (i & !1);
            let word = self.ioport(port);
            let (shift, keep) = if i & 1 == 0 { (0, 0xFF00u16) } else { (8, 0x00FFu16) };
            let byte = ((word >> shift) as u8).wrapping_add(1);
            self.set_ioport(port, (word & keep) | (u16::from(byte) << shift));
        }
    }

    /// Increments a slot's retry/TX counter (saturating at `0xFF`, with the
    /// upper byte cleared). Ported from `IncrementTXCount`
    /// (`Wifi.cpp:587-592`).
    fn increment_tx_count(&mut self, addr: u16) {
        let addr = addr as usize;
        let cnt = self.ram.get(addr + 4).copied().unwrap_or(0);
        self.set_ram_u16(addr + 4, u16::from(cnt.saturating_add(1)));
    }

    /// Advances the **latched** TX slot by one 8µs timer tick.
    ///
    /// The slot is chosen once, by [`Wifi::tick`], on the idle → transmitting
    /// transition, and re-chosen by [`Wifi::reselect_tx_slot`] only after one
    /// finishes. Ported from `USTimer` (`Wifi.cpp:1833-1849`); see
    /// `docs/design/local-mp-melonds-parity-2.md` F3 for why re-scanning per
    /// tick broke MP: the beacon slot (index 4) arms every beacon interval and
    /// would preempt an in-flight CMD round (index 1) mid-phase, silently
    /// stopping the phase-2 `mp_reply_timer` loop that delivers client replies.
    pub(super) fn process_tx(&mut self, request: &mut InterruptRequest) {
        let slot = match usize::try_from(self.tx_cur_slot) {
            Ok(slot) if slot < self.tx_slots.len() => slot,
            _ => {
                self.com_status &= !0x2;
                return;
            }
        };

        self.tx_slots[slot].phase_time -= super::Wifi::TIMER_INTERVAL_US as i32;

        if self.tx_slots[slot].phase_time > 0 {
            // Phase 2 (host reply-collection window) keeps polling replies
            // in as they arrive, paced by `mp_reply_timer`, exactly as
            // melonDS's `MPReplyTimer` countdown inside `ProcessTX`'s
            // `CurPhaseTime > 0` branch (`Wifi.cpp:955-971`). This is what
            // actually delivers client replies back to the host's game --
            // see `docs/design/local-mp-melonds-parity.md` Gap 3.3.
            if self.tx_slots[slot].phase == 2 {
                self.mp_reply_timer -= super::Wifi::TIMER_INTERVAL_US as i32;
                if self.mp_reply_timer <= 0 && self.mp_client_mask != 0 {
                    let nclient = (1..16u16).find(|&i| self.mp_client_mask & (1 << i) != 0);
                    if let Some(nclient) = nclient {
                        let curclient = 1u16 << nclient;
                        if self.mp_client_fail & curclient == 0 {
                            self.mp_client_reply_rx(nclient, request);
                        }
                        self.mp_reply_timer += 10 + self.ioport(W_CmdReplyTime) as i32;
                        self.mp_client_mask &= !curclient;
                    }
                }
            }
            return;
        }

        match self.tx_slots[slot].phase {
            0 => self.tx_phase_preamble_done(slot, request),
            10 => self.tx_phase_default_reply_preamble_done(slot, request),
            1 => self.tx_phase_transmit_done(slot, request),
            11 => self.tx_phase_default_reply_done(slot, request),
            2 => self.tx_phase_mp_host_done(slot, request),
            3 => self.tx_phase_mp_ack_done(slot, request),
            13 => self.tx_phase_cmd_timeout(slot, request),
            _ => {}
        }
    }

    fn tx_phase_preamble_done(&mut self, slot: usize, request: &mut InterruptRequest) {
        self.raise_irq(7, request);
        // Hardware points the RX/TX address register at the frame being sent
        // for the duration of the transfer (`Wifi.cpp:1004`).
        self.set_ioport(W_RXTXAddr, self.tx_slots[slot].addr >> 1);
        // Transmitting (`Wifi.cpp:983-985`).
        self.set_status(if slot == 5 { 8 } else { 3 });
        // Slot 5 (reply) already transmitted synchronously inside
        // `start_mp_reply`; sending it again here would duplicate the
        // frame. Ported from `ProcessTX`'s `if (num != 5) TXSendFrame(...)`.
        if slot != 5 && self.cur_channel != 0 {
            self.send_slot_frame(slot);
        }
        let s = self.tx_slots[slot];
        self.tx_slots[slot].phase = 1;
        self.tx_slots[slot].phase_time = s.length as i32 * if s.rate == 2 { 4 } else { 8 };
    }

    fn tx_phase_default_reply_preamble_done(
        &mut self,
        _slot: usize,
        request: &mut InterruptRequest,
    ) {
        self.raise_irq(7, request);
        self.set_status(8);
        self.tx_slots[5].phase = 11;
        self.tx_slots[5].phase_time = 28 * 4;
    }

    fn tx_phase_transmit_done(&mut self, slot: usize, request: &mut InterruptRequest) {
        let addr = self.tx_slots[slot].addr;
        if slot != 1 && slot != 5 {
            self.set_ram_u16(addr as usize, 0x0001);
        }
        if let Some(byte) = self.ram.get_mut(addr as usize + 5) {
            *byte = 0;
        }

        match slot {
            1 => {
                if self.ioport(W_TXStatCnt) & 0x4000 != 0 {
                    self.set_ioport(W_TXStat, 0x0800);
                    self.raise_irq(1, request);
                }
                // Waiting for client replies (`Wifi.cpp:1058`).
                self.set_status(5);
                self.mp_reply_timer = 16 + self.preamble_len(self.tx_slots[1].rate);
                if super::debug_enabled() {
                    eprintln!("[wifi] tx_phase_transmit_done us_timestamp={}", self.us_timestamp);
                }
                if self.mp_client_mask != 0
                    && let Some(mut transport) = self.transport.take()
                {
                    let mut buf = vec![0u8; self.mp_client_replies.len()];
                    let answered =
                        transport.recv_replies(&mut buf, self.us_timestamp, self.mp_client_mask);
                    if answered != 0 {
                        self.diag.replies_answered += 1;
                    } else {
                        self.diag.replies_empty += 1;
                    }
                    if super::debug_enabled() {
                        eprintln!("[wifi] recv_replies answered=0x{answered:04X}");
                    }
                    self.mp_client_replies[..buf.len()].copy_from_slice(&buf);
                    self.mp_client_fail &= !answered;
                    self.transport = Some(transport);
                }
                self.tx_slots[1].phase = 2;
                self.tx_slots[1].phase_time = 112
                    + (10 + self.ioport(W_CmdReplyTime) as i32)
                        * num_clients(self.mp_client_mask) as i32;
            }
            5 => {
                if self.ioport(W_TXStatCnt) & 0x1000 != 0 {
                    self.set_ioport(W_TXStat, 0x0401);
                    self.raise_irq(1, request);
                }
                self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !0x0080);
                self.tx_slots[5].valid = false;
                self.fire_tx();
                self.reselect_tx_slot();
            }
            _ => {
                self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !SLOT_BUSY_BITS[slot]);
                match slot {
                    0 | 2 | 3 => {
                        let loc = if slot == 0 { 0 } else { slot - 1 };
                        self.set_ioport(W_TXStat, 0x0001 | ((loc as u16) << 12));
                        self.raise_irq(1, request);
                        let reg = W_TXSlotLoc1 + loc * 4;
                        self.set_ioport(reg, self.ioport(reg) & 0x7FFF);
                    }
                    4 if self.ioport(W_TXStatCnt) & 0x8000 != 0 => {
                        self.set_ioport(W_TXStat, 0x0301);
                        self.raise_irq(1, request);
                    }
                    _ => {}
                }
                self.tx_slots[slot].valid = false;
                self.fire_tx();
                self.reselect_tx_slot();
            }
        }
    }

    fn tx_phase_default_reply_done(&mut self, _slot: usize, request: &mut InterruptRequest) {
        self.set_ioport(W_TXSeqNo, (self.ioport(W_TXSeqNo) + 1) & 0x0FFF);
        self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !0x0080);
        self.tx_slots[5].valid = false;
        let _ = request;
        self.fire_tx();
        self.reselect_tx_slot();
    }

    /// Phase 2: MP host command finished transmitting; the reply-collection
    /// window has now also elapsed. Broadcasts the acknowledgement and
    /// reports which clients (if any) failed to reply. Ported from
    /// `ProcessTX` case 2 (`Wifi.cpp:1125-1143`).
    fn tx_phase_mp_host_done(&mut self, _slot: usize, request: &mut InterruptRequest) {
        self.raise_irq(7, request);
        self.set_status(8);
        // Hardware parks the RX/TX pointer here for the ack window
        // (`Wifi.cpp:1131`).
        self.set_ioport(W_RXTXAddr, 0xFC0);
        let rate_factor = if self.tx_slots[1].rate == 2 { 4 } else { 8 };
        self.tx_slots[1].phase_time = 32 * rate_factor;

        // Bump each failed client's counter before the ack goes out, matching
        // `Wifi.cpp:1138-1141`'s order. melonDS leaves it unresolved whether a
        // reply failure raises any IRQ of its own (`Wifi.cpp:594-596`), so
        // neither does this port.
        self.report_mp_reply_errors(self.mp_client_fail);

        let cmdcount = self.cmd_counter.div_ceil(10) as u16;
        self.send_mp_ack(cmdcount);

        self.tx_slots[1].phase = 3;
    }

    /// Phase 3: the acknowledgement finished transmitting. Writes the CMD
    /// slot's TX status words, raises **IRQ 12** (MP transaction complete
    /// -- the interrupt the game's driver actually blocks on), and re-fires
    /// any queued transmit. Ported from `ProcessTX` case 3
    /// (`Wifi.cpp:1145-1183`); the automatic-retry branch on client failure
    /// is disabled upstream too (`&& false`) and is not ported.
    fn tx_phase_mp_ack_done(&mut self, _slot: usize, request: &mut InterruptRequest) {
        let addr = self.tx_slots[1].addr;
        if self.mp_client_fail == 0 {
            self.set_ram_u16(addr as usize, 0x0001);
        } else {
            self.set_ram_u16(addr as usize, 0x0005);
        }
        self.set_ram_u16(addr as usize + 0x2, self.mp_client_fail);
        if self.mp_client_fail == 0 {
            self.increment_tx_count(addr);
        }

        self.set_ioport(W_TXSeqNo, (self.ioport(W_TXSeqNo) + 1) & 0x0FFF);

        if self.ioport(W_TXStatCnt) & 0x2000 != 0 {
            self.set_ioport(W_TXStat, 0x0B01);
            self.raise_irq(1, request);
        }

        self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !(1 << 1));
        self.set_ioport(W_TXSlotCmd, self.ioport(W_TXSlotCmd) & 0x7FFF);
        self.tx_slots[1].valid = false;

        if super::debug_enabled() {
            eprintln!(
                "[wifi] MP CMD round complete: raising IRQ 12, clientfail=0x{:04X}",
                self.mp_client_fail
            );
        }
        self.diag.irq12 += 1;
        self.raise_irq(12, request);
        self.fire_tx();
        self.reselect_tx_slot();
    }

    /// Phase 13: the CMD slot started, but `CmdCounter` ran out before a
    /// full round could fit. Ported from `ProcessTX` case 13
    /// (`Wifi.cpp:1185-1199`). See
    /// `docs/design/local-mp-melonds-parity.md` Gap 1.4.
    fn tx_phase_cmd_timeout(&mut self, _slot: usize, request: &mut InterruptRequest) {
        self.set_ioport(W_TXBusy, self.ioport(W_TXBusy) & !(1 << 1));
        self.set_ioport(W_TXSlotCmd, self.ioport(W_TXSlotCmd) & 0x7FFF);
        let addr = self.tx_slots[1].addr;
        self.set_ram_u16(addr as usize, 0x0005);
        self.set_ioport(W_TXSeqNo, (self.ioport(W_TXSeqNo) + 1) & 0x0FFF);
        self.tx_slots[1].valid = false;
        self.diag.irq12 += 1;
        self.raise_irq(12, request);
        self.fire_tx();
        self.reselect_tx_slot();
    }

    /// Re-selects the current TX slot after one has finished, dropping back to
    /// idle when none is left. Ported from `USTimer`'s post-`ProcessTX`
    /// re-selection (`Wifi.cpp:1866-1881`).
    fn reselect_tx_slot(&mut self) {
        // Back to idle before picking the next slot (`Wifi.cpp:1081`,
        // `1109`, `1120`, `1177`, `1194`).
        self.set_status(1);
        match pick_busy_slot(self.ioport(W_TXBusy)) {
            Some(slot) => self.tx_cur_slot = slot as i32,
            None => {
                self.tx_cur_slot = -1;
                self.com_status &= !0x2;
                self.rx_counter = 0;
            }
        }
    }

    /// Builds the 12-byte hardware header + frame body into `tx_buffer` and
    /// hands it to the transport. Ported from `TXSendFrame`
    /// (`Wifi.cpp:607-678`); see
    /// `docs/design/local-mp-melonds-parity.md` Gap 4.2.
    ///
    /// Not ported: WEP FCS patching (out of scope, no WEP support) and the
    /// self-deauth informational log line (no behavioural effect upstream).
    fn send_slot_frame(&mut self, slot: usize) {
        debug_assert_ne!(slot, 5, "slot 5 transmits synchronously from `start_mp_reply`");
        let s = self.tx_slots[slot];
        let addr = s.addr as usize;

        // Sequence-number handling (Gap 4.2): a retransmit (retry counter
        // already nonzero) or an explicit CMD "no-seqno" request skips
        // incrementing/embedding a fresh sequence number.
        let retransmit = self.ram.get(addr + 4).copied().unwrap_or(0) != 0;
        let cmd_no_seqno = slot == 1 && self.ioport(W_TXSlotCmd) & 0x4000 != 0;
        let noseqno = if retransmit {
            2
        } else if cmd_no_seqno {
            1
        } else {
            0
        };

        if noseqno == 0 {
            if self.ioport(W_TXHeaderCnt) & (1 << 2) == 0 {
                let seq = self.ioport(W_TXSeqNo) << 4;
                self.set_ram_u16(addr + 0xC + 22, seq);
            }
            self.set_ioport(W_TXSeqNo, (self.ioport(W_TXSeqNo) + 1) & 0x0FFF);
        }

        // WEP frame: plant a nonzero WEP FCS. melonDS does no real WEP
        // processing either, but notes that "some games require it"
        // (`Wifi.cpp:627-640`) -- shared-key authentication carries an
        // encrypted challenge, and a zero FCS makes the peer reject the
        // exchange. Without this the handshake reaches sequence 3 and then
        // fails.
        let frame_ctl = self.ram_u16(addr + 0xC);
        if frame_ctl & (1 << 14) != 0 && self.ioport(W_WEPCnt) & (1 << 15) != 0 {
            let fcs = (addr + 0xC + s.length as usize).saturating_sub(7) & !0x1;
            if fcs + 3 < self.ram.len() {
                self.ram[fcs..fcs + 4].copy_from_slice(&0x2233_4466u32.to_le_bytes());
            }
        }

        let max_len = 0x1FF4usize.saturating_sub(addr);
        let len = (s.length as usize).min(max_len).min(Wifi::TX_BUFFER_SIZE.saturating_sub(12));
        let src_end = (addr + 12 + len).min(self.ram.len());
        let src_start = addr.min(src_end);
        let copy_len = src_end - src_start;
        self.tx_buffer[..copy_len].copy_from_slice(&self.ram[src_start..src_end]);

        if noseqno == 2 && copy_len >= 0xE {
            let fc = self.tx_buffer[0xC] as u16 | (self.tx_buffer[0xD] as u16) << 8 | (1 << 11);
            self.tx_buffer[0xC] = fc as u8;
            self.tx_buffer[0xD] = (fc >> 8) as u8;
        }

        if self.cur_channel == 0 {
            return;
        }
        // Only the channel byte is patched. The rate (`[8]`) and frame length
        // (`[0xA]`) are the game's own staged TX-header values and travel
        // verbatim -- melonDS overwrites neither (`Wifi.cpp:655-660`).
        // Re-deriving them here discarded whatever the game actually set. See
        // `docs/design/local-mp-melonds-parity-2.md` F8a.
        self.tx_buffer[9] = self.cur_channel as u8;

        if matches!(slot, 0 | 2 | 3) && copy_len >= 0xE {
            let fc = self.tx_buffer[0xC] as u16 | (self.tx_buffer[0xD] as u16) << 8;
            if fc & 0x00FF == 0x00C0 && self.is_mp_client {
                self.is_mp = false;
                self.is_mp_client = false;
            }
        }

        if super::debug_enabled() {
            let frame_ctl = self.tx_buffer.get(12).copied().unwrap_or(0) as u16
                | (self.tx_buffer.get(13).copied().unwrap_or(0) as u16) << 8;
            eprintln!(
                "[wifi] TX slot={slot} channel={} len={copy_len} frame_ctl=0x{frame_ctl:04X} \
                 transport_installed={}",
                self.cur_channel,
                self.transport.is_some()
            );
        }

        match slot {
            1 => {
                // Embed the (post-failure-masking) effective client mask,
                // matching melonDS writing `MPClientMask` into the
                // transmitted buffer at send time rather than trusting
                // whatever the game originally staged.
                if copy_len >= 12 + 24 + 4 {
                    let o = 12 + 24 + 2;
                    self.tx_buffer[o] = self.mp_client_mask as u8;
                    self.tx_buffer[o + 1] = (self.mp_client_mask >> 8) as u8;
                }
                let Some(mut transport) = self.transport.take() else { return };
                transport.send_cmd(&self.tx_buffer[..copy_len], self.us_timestamp);
                self.transport = Some(transport);
            }
            4 => {
                // Beacon: embed the host's microsecond sync counter so a
                // receiving client can adopt it. See
                // `docs/design/local-mp-melonds-parity.md` Gap 4.3.
                if copy_len >= 12 + 24 + 8 {
                    let o = 12 + 24;
                    self.tx_buffer[o..o + 8].copy_from_slice(&self.us_counter.to_le_bytes());
                }
                let Some(mut transport) = self.transport.take() else { return };
                transport.send_packet(&self.tx_buffer[..copy_len], self.us_timestamp);
                self.transport = Some(transport);
            }
            _ => {
                // LOC1-3: authentication/association and any other frame the
                // driver stages by hand. This is the guest's entire outbound
                // traffic until it associates.
                self.diag.loc_tx += 1;
                let Some(mut transport) = self.transport.take() else { return };
                transport.send_packet(&self.tx_buffer[..copy_len], self.us_timestamp);
                self.transport = Some(transport);
            }
        }
    }

    /// Builds and sends the client's staged MP reply frame. Ported from the
    /// "valid" branch of `SendMPReply`/`TXSendFrame` case 5
    /// (`Wifi.cpp:835-894`, `668-671`).
    fn send_mp_reply(&mut self, rate: u8) {
        let aid = self.ioport(W_AIDLow);
        let s = self.tx_slots[5];
        let addr = s.addr as usize;
        let len = (s.length as usize).min(Wifi::TX_BUFFER_SIZE.saturating_sub(12));
        let src_end = (addr + 12 + len).min(self.ram.len());
        let src_start = addr.min(src_end);
        let copy_len = src_end - src_start;
        self.tx_buffer[..copy_len].copy_from_slice(&self.ram[src_start..src_end]);

        if self.cur_channel == 0 {
            return;
        }
        // As in `Wifi::send_slot_frame`, only the channel byte is patched:
        // melonDS reaches the reply slot through the same `TXSendFrame`
        // (`SendMPReply` calls `TXSendFrame(slot, 5)`, and `ProcessTX` case 0
        // skips it precisely because it already ran), which overwrites neither
        // the rate nor the length. See
        // `docs/design/local-mp-melonds-parity-2.md` F8a.
        let _ = rate;
        self.tx_buffer[9] = self.cur_channel as u8;

        if super::debug_enabled() {
            eprintln!(
                "[wifi] TX mp_reply (staged, {copy_len} bytes) aid={aid} channel={}",
                self.cur_channel
            );
        }

        self.diag.reply_tx += 1;
        self.increment_tx_count(s.addr);
        let Some(mut transport) = self.transport.take() else { return };
        transport.send_reply(&self.tx_buffer[..copy_len], self.us_timestamp, aid);
        self.transport = Some(transport);
    }

    /// Sends a fixed 40-byte blank reply frame naming this client as
    /// `MPReplyMAC`, so the host does not time out. Used when the client
    /// has nothing staged, or its staged frame does not fit the reply-time
    /// budget. Ported from `SendMPDefaultReply` (`Wifi.cpp:802-833`); see
    /// `docs/design/local-mp-melonds-parity.md` §3.4 for the exact byte
    /// layout and Gap 3.1/3.2.
    fn send_mp_default_reply(&mut self) {
        let aid = self.ioport(W_AIDLow);
        if self.cur_channel == 0 {
            return;
        }

        let bssid0 = self.ioport(W_BSSID0);
        let bssid1 = self.ioport(W_BSSID1);
        let bssid2 = self.ioport(W_BSSID2);
        let mac0 = self.ioport(W_MACAddr0);
        let mac1 = self.ioport(W_MACAddr1);
        let mac2 = self.ioport(W_MACAddr2);
        let seqno = self.ioport(W_TXSeqNo) << 4;

        let buf = &mut self.tx_buffer[..12 + 28];
        buf.fill(0);
        buf[0x8] = 0x14;
        buf[0x9] = self.cur_channel as u8;
        buf[0xA] = 28;

        let write_u16 = |buf: &mut [u8], off: usize, value: u16| {
            buf[off] = value as u8;
            buf[off + 1] = (value >> 8) as u8;
        };
        write_u16(buf, 0xC, 0x0158);
        write_u16(buf, 0xC + 0x02, 0x00F0);
        write_u16(buf, 0xC + 0x04, bssid0);
        write_u16(buf, 0xC + 0x06, bssid1);
        write_u16(buf, 0xC + 0x08, bssid2);
        write_u16(buf, 0xC + 0x0A, mac0);
        write_u16(buf, 0xC + 0x0C, mac1);
        write_u16(buf, 0xC + 0x0E, mac2);
        // `MPReplyMAC` (03 09 BF 00 00 10) at address 3.
        write_u16(buf, 0xC + 0x10, 0x0903);
        write_u16(buf, 0xC + 0x12, 0x00BF);
        write_u16(buf, 0xC + 0x14, 0x1000);
        write_u16(buf, 0xC + 0x16, seqno);

        if super::debug_enabled() {
            eprintln!("[wifi] TX mp_reply (default/blank) aid={aid} channel={}", self.cur_channel);
        }

        self.diag.blank_reply_tx += 1;
        let Some(mut transport) = self.transport.take() else { return };
        transport.send_reply(&self.tx_buffer[..12 + 28], self.us_timestamp, aid);
        self.transport = Some(transport);
    }

    /// Broadcasts the host acknowledgement frame closing an MP CMD round,
    /// carrying the run-ahead window clients may consume before their next
    /// mandatory sync point. Ported from `SendMPAck` (`Wifi.cpp:896-943`);
    /// see `docs/design/local-mp-melonds-parity.md` §3.5 for the exact byte
    /// layout and run-ahead formula.
    pub(super) fn send_mp_ack(&mut self, cmdcount: u16) {
        if self.cur_channel == 0 {
            return;
        }
        let clientfail = self.mp_client_fail;
        let rate = self.tx_slots[1].rate;
        let bssid0 = self.ioport(W_BSSID0);
        let bssid1 = self.ioport(W_BSSID1);
        let bssid2 = self.ioport(W_BSSID2);
        let mac0 = self.ioport(W_MACAddr0);
        let mac1 = self.ioport(W_MACAddr1);
        let mac2 = self.ioport(W_MACAddr2);
        let seqno = self.ioport(W_TXSeqNo) << 4;

        let rate_factor = if rate == 2 { 4 } else { 8 };
        let runahead: i32 = if clientfail == 0 {
            let nextbeacon = if self.ioport(W_TXBusy) & 0x0010 != 0 {
                0
            } else {
                ((i32::from(self.ioport(W_BeaconCount1)) - 1) << 10)
                    + (0x400 - (self.us_counter & 0x3FF) as i32)
            };
            let mut runahead = self.cmd_counter.min(nextbeacon.max(0) as u32) as i32;
            if self.cmd_counter < 1000 {
                runahead -= 210;
            }
            (runahead - 32 * rate_factor).max(0)
        } else {
            self.preamble_len(rate)
        };

        let buf = &mut self.tx_buffer[..12 + 32];
        buf.fill(0);
        buf[0x8] = if rate == 2 { 0x14 } else { 0x0A };
        buf[0x9] = self.cur_channel as u8;
        buf[0xA] = 32;

        let write_u16 = |buf: &mut [u8], off: usize, value: u16| {
            buf[off] = value as u8;
            buf[off + 1] = (value >> 8) as u8;
        };
        write_u16(buf, 0xC, 0x0218);
        // `MPAckMAC` (03 09 BF 00 00 03) at address 1.
        write_u16(buf, 0xC + 0x04, 0x0903);
        write_u16(buf, 0xC + 0x06, 0x00BF);
        write_u16(buf, 0xC + 0x08, 0x0300);
        write_u16(buf, 0xC + 0x0A, bssid0);
        write_u16(buf, 0xC + 0x0C, bssid1);
        write_u16(buf, 0xC + 0x0E, bssid2);
        write_u16(buf, 0xC + 0x10, mac0);
        write_u16(buf, 0xC + 0x12, mac1);
        write_u16(buf, 0xC + 0x14, mac2);
        write_u16(buf, 0xC + 0x16, seqno);
        write_u16(buf, 0xC + 0x18, cmdcount);
        write_u16(buf, 0xC + 0x1A, clientfail);

        buf[0] = runahead as u8;
        buf[1] = (runahead >> 8) as u8;
        buf[2] = (runahead >> 16) as u8;
        buf[3] = (runahead >> 24) as u8;

        let Some(mut transport) = self.transport.take() else { return };
        transport.send_ack(&self.tx_buffer[..12 + 32], self.us_timestamp, runahead.max(0) as u32);
        self.transport = Some(transport);
    }
}
