//! The RAM search pane's scanning, over main RAM.

use super::*;

impl MelonEgui {
    /// Read the value at `addr` at the search's current width.
    pub fn ram_read(&mut self, addr: u32) -> u32 {
        let Some(emu) = &mut self.emu else { return 0 };
        match self.ram_search.width {
            crate::ui::panes::SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
            crate::ui::panes::SearchWidth::Half => u32::from(emu.nds.read16(addr)),
            crate::ui::panes::SearchWidth::Word => emu.nds.read32(addr),
        }
    }

    /// Scan the whole of main RAM for the value, replacing any previous results.
    pub fn ram_first_scan(&mut self) {
        let Some(needle) = self.ram_search.parse_needle() else { return };
        let width = self.ram_search.width;
        let Some(emu) = &mut self.emu else { return };

        // Main RAM starts at 0200_0000h on both CPUs (GBATEK, "Memory Maps").
        const MAIN_RAM_BASE: u32 = 0x0200_0000;
        let len = emu.nds.main_ram().len();
        let mut hits = Vec::new();
        let stride = width.size();
        for offset in (0..len.saturating_sub(stride - 1)).step_by(stride) {
            let addr = MAIN_RAM_BASE + offset as u32;
            let value = match width {
                crate::ui::panes::SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
                crate::ui::panes::SearchWidth::Half => u32::from(emu.nds.read16(addr)),
                crate::ui::panes::SearchWidth::Word => emu.nds.read32(addr),
            };
            if value == needle {
                hits.push(addr);
            }
        }
        let found = hits.len();
        self.ram_search.hits = hits;
        self.post(format!("RAM search: {found} addresses hold {needle}"));
    }

    /// Keep only the addresses that still hold the value.
    pub fn ram_narrow(&mut self) {
        let Some(needle) = self.ram_search.parse_needle() else { return };
        let width = self.ram_search.width;
        let Some(emu) = &mut self.emu else { return };

        let before = self.ram_search.hits.len();
        self.ram_search.hits.retain(|&addr| {
            let value = match width {
                crate::ui::panes::SearchWidth::Byte => u32::from(emu.nds.read8(addr)),
                crate::ui::panes::SearchWidth::Half => u32::from(emu.nds.read16(addr)),
                crate::ui::panes::SearchWidth::Word => emu.nds.read32(addr),
            };
            value == needle
        });
        let after = self.ram_search.hits.len();
        self.post(format!("RAM search: narrowed {before} to {after}"));
    }

    // -- commands -----------------------------------------------------------
}
