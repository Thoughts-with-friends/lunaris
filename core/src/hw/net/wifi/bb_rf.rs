//! Baseband (BB) and radio-frequency (RF) register transfer, and RF-driven
//! channel detection.
//!
//! Games select a Wi-Fi channel by writing calibration values (taken from
//! the firmware's Wi-Fi config block) into two "channel index" RF
//! registers. There is no direct "set channel N" register — the emulator
//! must recognize which of the 14 channel presets those two values match,
//! exactly as melonDS does (`docs/design/melonds/WiFi.cpp:1938-1997`).
//!
//! This is also where trap 3 from `docs/design/design_lan.md` §3.2 lives:
//! if the firmware calibration table were all zeros, writing zero to both
//! index registers would spuriously match channel 1. The synthetic
//! firmware config (§7.2) uses distinct, non-zero values per channel so a
//! zeroed/uninitialized RF resolves to channel 0 (`change_channel_rejects_zero`
//! unit test in `mod.rs`).

use super::{Wifi, regs::*};

impl Wifi {
    /// Compares the two channel-index RF registers against the firmware
    /// calibration table and updates `cur_channel` (0 = no match).
    pub(super) fn change_channel(&mut self) {
        let val1 = self.rf_regs[self.rf_channel_index[0] as usize % self.rf_regs.len()];
        let val2 = self.rf_regs[self.rf_channel_index[1] as usize % self.rf_regs.len()];

        let previous = self.cur_channel;
        self.cur_channel = 0;
        for (i, &[c1, c2]) in self.rf_channel_data.iter().enumerate() {
            if val1 == c1 && val2 == c2 {
                self.cur_channel = i as i32 + 1;
                break;
            }
        }
        if super::debug_enabled() && self.cur_channel != previous {
            eprintln!(
                "[wifi] channel resolved: {previous} -> {} (rf_regs[idx0]=0x{val1:X} rf_regs[idx1]=0x{val2:X})",
                self.cur_channel
            );
        }
    }

    /// Dispatches an RF register transfer triggered by a `W_RFData2` write.
    /// Type-2 and Type-3 RF chips use different bit layouts for the
    /// register id and read/write direction
    /// (`docs/design/melonds/WiFi.cpp:1960-1997`); lunaris's synthesized
    /// firmware always reports Type-3 (`docs/design/design_lan.md` §7.2),
    /// but Type-2 is kept for compatibility with imported real firmware
    /// dumps.
    pub(super) fn rf_transfer(&mut self) {
        if self.rf_version == 3 {
            self.rf_transfer_type3();
        } else {
            self.rf_transfer_type2();
        }
    }

    fn rf_transfer_type2(&mut self) {
        let data2 = self.ioport(W_RFData2);
        let id = ((data2 >> 2) & 0x1F) as usize;
        if data2 & 0x0080 != 0 {
            let data = self.rf_regs[id];
            self.set_ioport(W_RFData1, (data & 0xFFFF) as u16);
            let new_data2 = (data2 & 0xFFFC) | ((data >> 16) & 0x3) as u16;
            self.set_ioport(W_RFData2, new_data2);
        } else {
            let data1 = self.ioport(W_RFData1) as u32;
            let data = data1 | (((data2 & 0x0003) as u32) << 16);
            self.rf_regs[id] = data;
            if id as u32 == self.rf_channel_index[0] || id as u32 == self.rf_channel_index[1] {
                self.change_channel();
            }
        }
    }

    fn rf_transfer_type3(&mut self) {
        let data1 = self.ioport(W_RFData1);
        let data2 = self.ioport(W_RFData2);
        let id = ((data1 >> 8) & 0x3F) as usize;
        let cmd = data2 & 0xF;
        if cmd == 6 {
            let val = self.rf_regs[id] & 0xFF;
            self.set_ioport(W_RFData1, (data1 & 0xFF00) | val as u16);
        } else if cmd == 5 {
            let data = (data1 & 0xFF) as u32;
            self.rf_regs[id] = data;
            if id as u32 == self.rf_channel_index[0] || id as u32 == self.rf_channel_index[1] {
                self.change_channel();
            }
        }
    }
}
