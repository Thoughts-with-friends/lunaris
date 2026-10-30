//! Receive side: frames from the link into the RX ring buffer.
//!
//! ```text
//!  us_timer (idle, every 512 µs)  -> check_rx(Any)    link.recv_packet
//!  us_timer (MP client, at sync)  -> check_rx(Host)   link.recv_host_packet
//!  process_tx phase 2 (MP host)   -> mp_client_reply_rx(aid)
//!    |
//!    v
//!  start_rx()      IRQ6, the frame is written into W_RXBUF one halfword
//!  rx_tick() x N   per 4/8 µs, as fast as the radio would deliver it
//!  finish_rx()     address/RX filters, 12-byte RX header, W_RXBUF_WRCSR,
//!                  IRQ0; a MP CMD addressed to us is answered
//!                  (tx.rs send_mp_reply)
//! ```
//!
//! An MP client runs in lockstep with its host: a host frame is only
//! delivered once the client's clock reaches the timestamp it was sent with,
//! and the client stops at `next_sync` until the host's next frame arrives.

use super::{
    tx::{MP_ACK_MAC, MP_CMD_MAC, MP_REPLY_MAC},
    *,
};

/// Which receive [`WiFi::check_rx`] performs.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RxSource {
    /// Any frame, non-blocking.
    Any,
    /// MP client: the host's next frame (blocks up to the link's timeout).
    Host,
}

/// Is `a` a multicast/broadcast address.
const fn is_group(mac: &[u8]) -> bool {
    // DS Wifi IEEE802.11 Frames: https://problemkaputt.de/gbatek.htm#dswifiieee80211frames
    mac[0] & 0x01 != 0 // MAC bit 0 set: group/multicast address
}

impl WiFi {
    fn rx_u16(&self, offset: usize) -> u16 {
        u16::from_le_bytes([self.rx_buffer[offset], self.rx_buffer[offset + 1]])
    }

    fn rx_mac(&self, offset: usize) -> [u8; 6] {
        let mut mac = [0; 6];
        mac.copy_from_slice(&self.rx_buffer[offset..offset + 6]);
        mac
    }

    /// Advances an RX ring buffer address by `inc` bytes, wrapping from
    /// W_RXBUF_END to W_RXBUF_BEGIN.
    fn increment_rx_addr(&self, addr: &mut usize, inc: usize) {
        // DS Wifi Receive Buffer: https://problemkaputt.de/gbatek.htm#dswifireceivebuffer
        let end = usize::from(self.port(W_RXBUF_END)) & 0x1FFE; // halfword aligned Wifi RAM offset
        let begin = usize::from(self.port(W_RXBUF_BEGIN)) & 0x1FFE; // halfword aligned Wifi RAM offset
        for _ in (0..inc).step_by(2) {
            *addr = (*addr + 2) & 0x1FFE;
            if *addr == end {
                *addr = begin;
            }
        }
    }

    /// Whether the receiver is on (transceiver powered, RX enabled and a
    /// buffer configured).
    const fn rx_enabled(&self) -> bool {
        self.port(W_POWERSTATE) & (1 << 9) == 0
            // DS Wifi Receive Control: https://problemkaputt.de/gbatek.htm#dswifireceivecontrol
            && self.port(W_RXCNT) & 0x8000 != 0 // W_RXCNT bit 15: enable queuing received data to RX FIFO
            && self.port(W_RXBUF_BEGIN) != self.port(W_RXBUF_END)
    }

    /// Frame length after W_RXLEN_CROP (FCS / WEP fields the hardware
    /// strips); moves a WEP frame's body over its IV.
    fn crop_rx_frame(&mut self, framectl: u16, framelen: usize) -> usize {
        let crop = usize::from(self.port(W_RXLEN_CROP));
        if framectl & (1 << 14) != 0 {
            // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
            let len = framelen.saturating_sub((crop >> 7) & 0x1FE); // W_RXLEN_CROP bits 8-15: crop amount for one frame type (halfwords)
            if len > 24 {
                let end = (12 + 28 + len).min(self.rx_buffer.len());
                self.rx_buffer.copy_within(12 + 28..end, 12 + 24);
            }
            len
        } else {
            framelen.saturating_sub((crop << 1) & 0x1FE) // W_RXLEN_CROP bits 0-7: crop amount (halfwords)
        }
    }

    /// Starts writing the frame in `rx_buffer` into the RX ring buffer.
    pub(super) fn start_rx(&mut self) {
        let framelen = self.rx_u16(8);
        let rate = self.rx_u16(6);
        // DS Wifi Hardware Headers: https://problemkaputt.de/gbatek.htm#dswifihardwareheaders
        let (per_byte, mask) = if rate == 0x14 {
            // rate 14h = 2 Mbit/s
            (4, 0x7 & STEP_MASK as u32) // 2 Mbit/s: 4 us per byte
        } else {
            (8, 0xF & STEP_MASK as u32) // 1 Mbit/s: 8 us per byte
        };
        self.rx_time = i32::from(framelen) * per_byte;
        self.rx_halfword_time_mask = mask;

        let mut addr = usize::from(self.port(W_RXBUF_WRCSR)) << 1;
        self.increment_rx_addr(&mut addr, 12);
        *self.port_mut(W_RXTX_ADDR) = (addr >> 1) as u16;
        self.rx_buffer_ptr = 12;

        self.set_irq(6);
        self.set_status(6);
        self.com_status |= 1;
    }

    /// One RX step: the next halfword reaches Wi-Fi RAM.
    pub(super) fn rx_tick(&mut self) {
        self.rx_time -= US_STEP as i32;
        if self.rx_time as u32 & self.rx_halfword_time_mask != 0 {
            return;
        }

        let mut addr = usize::from(self.port(W_RXTX_ADDR)) << 1;
        if addr < 0x1FFF {
            // inside 8K Wifi RAM
            let ptr = self.rx_buffer_ptr;
            let half =
                self.rx_buffer.get(ptr..ptr + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            self.set_ram_u16(addr, half);
        }
        self.increment_rx_addr(&mut addr, 2);
        *self.port_mut(W_RXTX_ADDR) = (addr >> 1) as u16;
        self.rx_buffer_ptr += 2;

        if self.rx_time <= 0 {
            self.finish_rx();
        } else if addr == usize::from(self.port(W_RXBUF_READCSR)) << 1 {
            // RX buffer full: drop the rest of the frame.
            #[cfg(feature = "log")]
            log::debug!("wifi: RX buffer full");
            self.rx_time = 0;
            self.set_status(1);
            if self.tx_cur_slot.is_none() {
                self.com_status &= !0x1; // communication state: receive done
                self.rx_counter = 0;
            }
            // DS Wifi Power-Down Registers: https://problemkaputt.de/gbatek.htm#dswifipowerdownregisters
            if self.com_status == 0 && self.port(W_POWERSTATE) & (1 << 9) != 0 {
                // W_POWERSTATE bit 9: powered down
                *self.port_mut(W_TRX_POWER) = 0;
                self.set_status(9);
            }
        }
    }

    /// The whole frame is in: filter it, write its RX header and signal it
    /// (melonDS `FinishRX`).
    fn finish_rx(&mut self) {
        self.com_status &= !0x1; // communication state: receive done
        self.rx_counter = 0;

        if self.com_status == 0 {
            if self.port(W_POWERSTATE) & (1 << 9) != 0 {
                // W_POWERSTATE bit 9: powered down
                *self.port_mut(W_TRX_POWER) = 0;
                self.set_status(9);
            } else {
                self.set_status(1);
            }
        }

        let framectl = self.rx_u16(12);
        let seqno = self.rx_u16(12 + 22);

        // The hardware checks the first address field whatever the frame.
        let dst = self.rx_mac(12 + 4);
        if !is_group(&dst) && dst != self.port_mac(W_MACADDR_0) {
            self.diag.drops.not_for_us += 1;
            return;
        }
        // WEP frames are dropped while WEP is off.
        if framectl & (1 << 14) != 0 && self.port(W_WEPCNT) & (1 << 15) == 0 {
            self.diag.drops.not_for_us += 1;
            return;
        }

        let Some((rxflags, cmd_dupe)) = self.rx_filter(framectl, seqno) else {
            self.diag.drops.filtered += 1;
            return;
        };
        self.diag.classify(rxflags);

        if !cmd_dupe {
            // RX header: flags, ?, rate, length, RSSI.
            let mut header = usize::from(self.port(W_RXBUF_WRCSR)) << 1;
            self.set_ram_u16(header, rxflags);
            self.increment_rx_addr(&mut header, 2);
            // DS Wifi Hardware Headers: https://problemkaputt.de/gbatek.htm#dswifihardwareheaders
            self.set_ram_u16(header, 0x0040); // RX header flags for a multiplay reply
            self.increment_rx_addr(&mut header, 4);
            self.set_ram_u16(header, self.rx_u16(6));
            self.increment_rx_addr(&mut header, 2);
            self.set_ram_u16(header, self.rx_u16(8));
            self.increment_rx_addr(&mut header, 2);
            self.set_ram_u16(header, 0x4080); // RX header flags for an ACK

            let mut addr = usize::from(self.port(W_RXTX_ADDR)) << 1;
            if addr & 0x2 != 0 {
                // keep the write pointer word-aligned
                self.increment_rx_addr(&mut addr, 2);
            }
            *self.port_mut(W_RXBUF_WRCSR) = ((addr & !0x3) >> 1) as u16;

            self.set_irq(0);
        }

        if rxflags & 0x800F == 0x800C {
            // MP CMD from our host: reply in our slot, or with a blank so
            // the host is not left waiting for a timeout.
            let client_mask = self.rx_u16(0xC + 26);
            let aid = self.port(W_AID_LOW);
            if aid != 0 && client_mask & (1 << aid) != 0 {
                self.send_mp_reply(self.rx_u16(0xC + 24), client_mask); // multiplay command: client time at frame offset 24
            } else if let Some(link) = self.link.as_mut() {
                link.send_reply(&[], self.us_timestamp, 0);
            }
        } else if rxflags & 0x800F == 0x8001 {
            // Beacon of our BSS: its timestamp is copied to W_US_COUNT.
            let rate = self.rx_u16(6);
            let len = u64::from(self.rx_u16(8)) * if rate == 0x14 { 4 } else { 8 }; // RX header +08h length; 4 or 8 us per byte
            let mut ts = [0; 8];
            ts.copy_from_slice(&self.rx_buffer[12 + 24..12 + 32]);
            self.us_counter = u64::from_le_bytes(ts).wrapping_add(len.wrapping_sub(76));
        }
    }

    /// RX filtering (W_RXFILTER / W_RXFILTER2). Returns the RX header flags
    /// and whether the frame is a duplicate MP CMD, or `None` to drop it.
    fn rx_filter(&mut self, framectl: u16, seqno: u16) -> Option<(u16, bool)> {
        let bssid = self.port_mac(W_BSSID_0);
        let filter = self.port(W_RXFILTER);
        // DS Wifi Hardware Headers: https://problemkaputt.de/gbatek.htm#dswifihardwareheaders
        let mut flags: u16 = 0x0010; // RX header flags: bit 4 always set
        let mut cmd_dupe = false;

        match (framectl >> 2) & 0x3 {
            // Management.
            0 => {
                if self.rx_mac(12 + 16) == bssid {
                    flags |= 0x8000; // bit 15: frame matches our BSSID
                }
                let subtype = (framectl >> 4) & 0xF; // frame control subtype (bits 4-7)
                if subtype == 0x8 {
                    // management subtype 8: beacon
                    if flags & 0x8000 == 0 && filter & 1 == 0 {
                        // W_RXFILTER bit 0: accept foreign beacons
                        return None;
                    }
                    flags |= 0x0001; // frame type 1: beacon
                } else if (subtype <= 0x5 || (0xA..=0xC).contains(&subtype)) // other management subtypes
                    && flags & 0x8000 == 0 // bit 15: frame matches our BSSID
                    && filter & (3 << 9) == 0
                {
                    return None;
                }
            }
            // Control: only PS-poll.
            1 => {
                if framectl & 0xF0 != 0xA0 {
                    // frame control A0h: disassociation
                    return None;
                }
                if self.rx_mac(12 + 4) == bssid {
                    flags |= 0x8000; // bit 15: frame matches our BSSID
                }
                if flags & 0x8000 == 0 && filter & (1 << 11) == 0 {
                    // W_RXFILTER bit 11: accept foreign frames
                    return None;
                }
                flags |= 0x0005; // frame type 5
            }
            // Data.
            2 => {
                let fromto = (framectl >> 8) & 0x3; // ToDS/FromDS bits (frame control bits 8-9)
                if self.port(W_RXFILTER2) & (1 << fromto) != 0 {
                    return None;
                }
                let bssid_offset = [16, 4, 10, 0][usize::from(fromto)];
                if bssid_offset != 0 && self.rx_mac(12 + bssid_offset) == bssid {
                    flags |= 0x8000; // bit 15: frame matches our BSSID
                }
                if flags & 0x8000 == 0 && filter & (1 << 11) == 0 {
                    // W_RXFILTER bit 11: accept foreign frames
                    return None;
                }
                // DS Wifi IEEE802.11 Frames: https://problemkaputt.de/gbatek.htm#dswifiieee80211frames
                if framectl & (1 << 11) != 0 && filter & 1 == 0 {
                    // frame control bit 11: retry; W_RXFILTER bit 0
                    return None; // retransmission
                }

                // MP frames are recognised by their addresses alone.
                if self.rx_mac(12 + 16) == MP_REPLY_MAC {
                    flags |= if framectl & 0xF0 == 0x50 {
                        // data subtype 5: CF-Ack (multiplay reply)
                        0x000F // frame type F: multiplay reply
                    } else {
                        0x000E // frame type E: multiplay ACK
                    };
                } else if self.rx_mac(12 + 4) == MP_CMD_MAC {
                    cmd_dupe = seqno == self.mp_last_seqno;
                    self.mp_last_seqno = seqno;
                    flags |= 0x000C; // frame type C: multiplay command
                } else if self.rx_mac(12 + 4) == MP_ACK_MAC {
                    flags |= 0x000D; // frame type D
                } else {
                    flags |= 0x0008; // frame type 8: data
                }

                let kind = flags & 0xF; // frame type (bits 0-3)
                let allowed = match (framectl >> 4) & 0xF {
                    // frame control subtype (bits 4-7)
                    0x0 | 0x4 => true, // always accepted
                    0x1 if kind == 0xD => filter & (1 << 7) != 0, // W_RXFILTER bit 7
                    0x1 => kind == 0xE || filter & (1 << 1) != 0, // W_RXFILTER bit 1
                    0x2 => kind == 0xC || filter & (1 << 2) != 0, // W_RXFILTER bit 2
                    0x3 => filter & (1 << 3) != 0, // W_RXFILTER bit 3
                    0x5 if kind == 0xF => filter & (1 << 8) != 0, // W_RXFILTER bit 8
                    0x5 => filter & (1 << 4) != 0, // W_RXFILTER bit 4
                    0x6 => filter & (1 << 5) != 0, // W_RXFILTER bit 5
                    0x7 => filter & (1 << 6) != 0, // W_RXFILTER bit 6
                    _ => false,
                };
                if !allowed {
                    return None;
                }
            }
            _ => {}
        }
        Some((flags, cmd_dupe))
    }

    /// MP host: client `client`'s reply slot came up; receive what it sent.
    pub(super) fn mp_client_reply_rx(&mut self, client: usize) {
        if !self.rx_enabled() {
            return;
        }
        let base = (client - 1) * 1024;
        let reply = &self.mp_client_replies[base..base + 1024];
        let framelen = usize::from(u16::from_le_bytes([reply[10], reply[11]]));
        let rate = reply[8];
        let framectl = u16::from_le_bytes([reply[12], reply[13]]);

        let copy = (12 + framelen).min(1024).min(self.rx_buffer.len());
        self.rx_buffer[..copy].copy_from_slice(&self.mp_client_replies[base..base + copy]);
        let framelen = self.crop_rx_frame(framectl, framelen);

        self.rx_buffer[6..8].copy_from_slice(&u16::from(rate).to_le_bytes());
        self.rx_buffer[8..10].copy_from_slice(&(framelen as u16).to_le_bytes());
        self.rx_timestamp = 0;
        self.start_rx();
    }

    /// Polls the link for a frame and starts (or, for a MP client, schedules)
    /// its reception (melonDS `CheckRX`). Returns whether one arrived.
    pub(super) fn check_rx(&mut self, source: RxSource) -> bool {
        if self.link.is_none() {
            return false;
        }
        self.diag.rx_polls += 1;
        if !self.rx_enabled() {
            self.diag.drops.rx_disabled += 1;
            return false;
        }
        let Some(link) = self.link.as_mut() else {
            return false;
        };
        let diag = &mut self.diag;

        let (framelen, timestamp) = loop {
            let recv = match source {
                RxSource::Any => link.recv_packet(&mut self.rx_buffer),
                RxSource::Host => link.recv_host_packet(&mut self.rx_buffer),
            };
            let (len, timestamp) = match recv {
                MpRecv::Frame { len, timestamp } => (len, timestamp),
                MpRecv::Empty => {
                    diag.rx_empty += 1;
                    return false;
                }
                MpRecv::HostGone => {
                    if source == RxSource::Host {
                        #[cfg(feature = "log")]
                        log::info!("wifi: MP host gone");
                        self.is_mp = false;
                        self.is_mp_client = false;
                    }
                    return false;
                }
            };
            if len < 12 + 24 || len > self.rx_buffer.len() {
                diag.drops.too_short += 1;
                continue;
            }
            let framelen =
                usize::from(u16::from_le_bytes([self.rx_buffer[10], self.rx_buffer[11]]));
            if framelen != len - 12 {
                diag.drops.bad_length += 1;
                continue;
            }
            if self.rx_buffer[9] != self.cur_channel || self.cur_channel == 0 {
                diag.drops.channel_mismatch += 1;
                continue;
            }
            // Ignore MP traffic while not part of a MP session.
            if source == RxSource::Any
                && !self.is_mp
                && (self.rx_buffer[12 + 16..12 + 22] == MP_REPLY_MAC
                    || self.rx_buffer[12 + 4..12 + 10] == MP_CMD_MAC
                    || self.rx_buffer[12 + 4..12 + 10] == MP_REPLY_MAC)
            {
                diag.drops.foreign_mp += 1;
                continue;
            }
            break (framelen, timestamp);
        };
        self.diag.rx_accepted += 1;

        let framectl = self.rx_u16(12);
        let rate = self.rx_buffer[8];
        let framelen = self.crop_rx_frame(framectl, framelen);
        self.rx_buffer[6..8].copy_from_slice(&u16::from(rate).to_le_bytes());
        self.rx_buffer[8..10].copy_from_slice(&(framelen as u16).to_le_bytes());

        // DS Wifi Hardware Headers: https://problemkaputt.de/gbatek.htm#dswifihardwareheaders
        let frame_us = framelen as u64 * if rate == 0x14 { 4 } else { 8 }; // 2 Mbit/s: 4 us per byte, 1 Mbit/s: 8 us
        let frametype = framectl & 0x00FF; // frame control low byte: type/subtype
        let dst = self.rx_mac(12 + 4);
        let mac_good = is_group(&dst) || dst == self.port_mac(W_MACADDR_0);

        // While not yet in lockstep, the auth/assoc handshake can lag behind
        // the host's post-beacon window: stretch that window.
        if matches!(frametype, 0x00B0 | 0x0010 | 0x0000)
            && timestamp != 0
            && mac_good
            && self.port(W_BEACON_COUNT2) != 0
        {
            *self.port_mut(W_BEACON_COUNT2) = self.port(W_BEACON_COUNT2).wrapping_add(10);
        }

        if frametype == 0x0010 {
            self.diag.last_assoc_aid = self.rx_u16(12 + 24 + 4);
            self.diag.last_assoc_mac_good = mac_good;
            self.diag.last_assoc_timestamp = timestamp;
            if trace::enabled() {
                self.assoc_trace_reads = 32;
                trace::log(format!(
                    "[assoc] mac {:02X?} got association response: aid {:04X} mac_good \
                     {mac_good} ts {timestamp} len {framelen} body {:02X?}",
                    self.port_mac(W_MACADDR_0),
                    self.diag.last_assoc_aid,
                    &self.rx_buffer[12..(12 + framelen).min(12 + 40)],
                ));
            }
        }
        if frametype == 0x0010 && timestamp != 0 && mac_good {
            // Association response: adopt the host's clock and go lockstep.
            let aid = self.rx_u16(12 + 24 + 4);
            if aid != 0 {
                #[cfg(feature = "log")]
                log::info!("wifi: associated as MP client {:X}", aid & 0xF); // AID (bits 0-3)
                self.is_mp = true;
                self.is_mp_client = true;
                self.us_timestamp = timestamp;
                self.next_sync = self.rx_timestamp + frame_us;
            }
            self.rx_timestamp = 0;
            self.start_rx();
        } else if frametype == 0x00C0 && timestamp != 0 && mac_good && self.is_mp_client {
            // Deauthentication from the host.
            self.is_mp = false;
            self.is_mp_client = false;
            self.next_sync = 0;
            self.rx_timestamp = 0;
            self.start_rx();
        } else if mac_good && self.is_mp_client {
            // Deliver at the host's timestamp, and allow running up to the
            // end of this frame (plus the reply window / the ACK's run-ahead).
            self.rx_timestamp = timestamp.max(self.us_timestamp);
            self.next_sync = self.rx_timestamp + frame_us;
            if self.rx_buffer[12 + 4..12 + 10] == MP_CMD_MAC {
                let client_time = u64::from(self.rx_u16(12 + 24));
                let client_mask = self.rx_u16(12 + 26);
                self.next_sync += 112 + (client_time + 10) * u64::from(client_mask.count_ones());
            } else if self.rx_buffer[12 + 4..12 + 10] == MP_ACK_MAC {
                let runahead = u32::from_le_bytes([
                    self.rx_buffer[0],
                    self.rx_buffer[1],
                    self.rx_buffer[2],
                    self.rx_buffer[3],
                ]);
                self.next_sync += u64::from(runahead);
            }
        } else {
            self.rx_timestamp = 0;
            self.start_rx();
        }
        true
    }
}
