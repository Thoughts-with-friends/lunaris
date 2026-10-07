//! The RAM search pane's scanning over the DS's 4 MB main RAM.
//!
//! First scan: every aligned address holding the value. Narrow: keep only the
//! addresses that still hold it (repeat while the value changes in-game).

use super::*;
use crate::ui::panes::SearchWidth;

/// Main RAM starts at 0200_0000h on both CPUs (GBATEK, "Memory Maps").
const MAIN_RAM_BASE: u32 = 0x0200_0000;

/// Read a value of `width` at `addr`.
fn read(emu: &mut Emu, width: SearchWidth, addr: u32) -> u32 {
    match width {
        SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
        SearchWidth::Half => u32::from(emu.nds.read16(addr)),
        SearchWidth::Word => emu.nds.read32(addr),
    }
}

impl MelonEgui {
    /// Read the value at `addr` at the search's current width.
    pub fn ram_read(&mut self, addr: u32) -> u32 {
        let width = self.ram_search.width;
        self.emu.as_mut().map_or(0, |emu| read(emu, width, addr))
    }

    /// Scan the whole of main RAM for the value, replacing any previous results.
    pub fn ram_first_scan(&mut self) {
        let Some(needle) = self.ram_search.parse_needle() else { return };
        let width = self.ram_search.width;
        let Some(emu) = &mut self.emu else { return };

        let len = emu.nds.main_ram().len();
        let stride = width.size();
        let hits: Vec<u32> = (0..len.saturating_sub(stride - 1))
            .step_by(stride)
            .map(|offset| MAIN_RAM_BASE + offset as u32)
            .filter(|&addr| read(emu, width, addr) == needle)
            .collect();
        let found = hits.len();
        self.ram_search.hits = hits;
        self.post(self.i18n().f(K::RamSearchFound, &[&found, &needle]));
    }

    /// Keep only the addresses that still hold the value.
    pub fn ram_narrow(&mut self) {
        let Some(needle) = self.ram_search.parse_needle() else { return };
        let width = self.ram_search.width;
        let Some(emu) = &mut self.emu else { return };

        let before = self.ram_search.hits.len();
        self.ram_search.hits.retain(|&addr| read(emu, width, addr) == needle);
        let after = self.ram_search.hits.len();
        self.post(self.i18n().f(K::RamSearchNarrowed, &[&before, &after]));
    }
}
