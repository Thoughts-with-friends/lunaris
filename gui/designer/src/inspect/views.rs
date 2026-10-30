//! Block-specific views besides registers: VRAM bank map and contents,
//! memory hex dumps, CPU state.

use ds_emu::Emulator;
use eframe::egui::{self, Color32, RichText};
use lunaris_gui_common::{console::Console, framebuffer::abgr1555_to_rgba8};

/// Bank sizes in KiB, A..I (GBATEK "DS Memory Control - VRAM").
const BANK_KIB: [u32; 9] = [128, 128, 128, 128, 64, 16, 16, 32, 16];

/// Where VRAMCNT value `cnt` maps bank `bank` (0 = A).
pub fn vram_mapping(bank: usize, cnt: u8) -> String {
    if cnt & 0x80 == 0 {
        return "disabled".to_owned();
    }
    let (mst, ofs) = (u32::from(cnt & 7), u32::from((cnt >> 3) & 3));
    let lcdc = [
        0x0680_0000,
        0x0682_0000,
        0x0684_0000,
        0x0686_0000,
        0x0688_0000,
        0x0689_0000,
        0x0689_4000,
        0x0689_8000,
        0x068A_0000,
    ];
    let fg_ofs = 0x4000 * (ofs & 1) + 0x1_0000 * (ofs >> 1);
    // Banks A and B ignore MST bit 2.
    match (bank, mst & if bank <= 1 { 3 } else { 7 }) {
        (_, 0) => format!("LCDC {:08X}", lcdc[bank]),
        (0..=3, 1) => format!("2D A BG {:08X}", 0x0600_0000 + 0x2_0000 * ofs),
        (0 | 1, 2) => format!("2D A OBJ {:08X}", 0x0640_0000 + 0x2_0000 * (ofs & 1)),
        (2 | 3, 2) => format!("ARM7 {:08X}", 0x0600_0000 + 0x2_0000 * (ofs & 1)),
        (0..=3, 3) => format!("3D texture slot {ofs}"),
        (2, 4) => "2D B BG 06200000".to_owned(),
        (3, 4) => "2D B OBJ 06600000".to_owned(),
        (4, 1) => "2D A BG 06000000".to_owned(),
        (4, 2) => "2D A OBJ 06400000".to_owned(),
        (4, 3) => "3D texture palette 0-3".to_owned(),
        (4, 4) => "2D A BG ext palette".to_owned(),
        (5 | 6, 1) => format!("2D A BG {:08X}", 0x0600_0000 + fg_ofs),
        (5 | 6, 2) => format!("2D A OBJ {:08X}", 0x0640_0000 + fg_ofs),
        (5 | 6, 3) => format!("3D texture palette slot {}", (ofs & 1) + (ofs >> 1) * 4),
        (5 | 6, 4) => {
            format!("2D A BG ext palette slots {}", if ofs & 1 == 0 { "0-1" } else { "2-3" })
        }
        (5 | 6, 5) => "2D A OBJ ext palette".to_owned(),
        (7, 1) => "2D B BG 06200000".to_owned(),
        (7, 2) => "2D B BG ext palette".to_owned(),
        (8, 1) => "2D B BG 06208000".to_owned(),
        (8, 2) => "2D B OBJ 06600000".to_owned(),
        (8, 3) => "2D B OBJ ext palette".to_owned(),
        _ => format!("MST {mst} (invalid)"),
    }
}

fn vramcnt(e: &Emulator, bank: usize) -> u8 {
    match bank {
        0 => e.gpu.get_vramcnt_a(),
        1 => e.gpu.get_vramcnt_b(),
        2 => e.gpu.get_vramcnt_c(),
        3 => e.gpu.get_vramcnt_d(),
        4 => e.gpu.get_vramcnt_e(),
        5 => e.gpu.get_vramcnt_f(),
        6 => e.gpu.get_vramcnt_g(),
        7 => e.gpu.get_vramcnt_h(),
        _ => e.gpu.get_vramcnt_i(),
    }
}

pub struct VramView {
    bank: usize,
    texture: Option<egui::TextureHandle>,
}

impl VramView {
    pub const fn new() -> Self {
        Self { bank: 0, texture: None }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, nds: &Console) {
        let e = nds.emulator();
        egui::Grid::new("vram_banks").striped(true).num_columns(4).show(ui, |ui| {
            for h in ["bank", "size", "VRAMCNT", "mapped to"] {
                ui.strong(h);
            }
            ui.end_row();
            for (bank, kib) in BANK_KIB.iter().enumerate() {
                let cnt = vramcnt(e, bank);
                let name = format!("{}", (b'A' + bank as u8) as char);
                if ui.selectable_label(self.bank == bank, name).clicked() {
                    self.bank = bank;
                }
                ui.label(format!("{kib} KiB"));
                ui.monospace(format!("{cnt:02X}"));
                let text = RichText::new(vram_mapping(bank, cnt));
                ui.label(if cnt & 0x80 != 0 {
                    text.color(Color32::from_rgb(90, 200, 120))
                } else {
                    text.weak()
                });
                ui.end_row();
            }
        });
        ui.separator();
        ui.label(
            RichText::new(format!(
                "Bank {} contents (as 16-bit colour)",
                (b'A' + self.bank as u8) as char
            ))
            .strong(),
        );
        let (pixels, w, h) = nds.render_bank(self.bank, true);
        if w == 0 || h == 0 {
            return;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &abgr1555_to_rgba8(&pixels));
        let tex = match &mut self.texture {
            Some(t) => {
                t.set(image, egui::TextureOptions::NEAREST);
                t
            }
            None => self.texture.insert(ui.ctx().load_texture(
                "vram_bank",
                image,
                egui::TextureOptions::NEAREST,
            )),
        };
        let width = ui.available_width().min(w as f32 * 2.0);
        ui.add(egui::Image::new((tex.id(), egui::vec2(width, width * h as f32 / w as f32))));
    }
}

/// A hex dump of one of the core's RAMs.
pub struct MemoryView {
    pub offset: usize,
    input: String,
}

impl MemoryView {
    pub const fn new() -> Self {
        Self { offset: 0, input: String::new() }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, mem: &[u8], base: u32) {
        ui.horizontal(|ui| {
            ui.label("Go to");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .desired_width(90.0)
                    .hint_text(format!("{base:08X}")),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(addr) =
                    u32::from_str_radix(self.input.trim().trim_start_matches("0x"), 16)
                {
                    self.offset = (addr.wrapping_sub(base) as usize % mem.len().max(1)) & !15;
                }
            }
            if ui.button("<<").clicked() {
                self.offset = self.offset.saturating_sub(0x100);
            }
            if ui.button(">>").clicked() {
                self.offset = (self.offset + 0x100).min(mem.len().saturating_sub(0x100));
            }
        });
        let rows = 24;
        let mut text = String::new();
        for row in 0..rows {
            let start = self.offset + row * 16;
            let Some(line) = mem.get(start..start + 16) else { break };
            text += &format!("{:08X}  ", base as usize + start);
            for (i, b) in line.iter().enumerate() {
                text += &format!("{b:02X}{}", if i == 7 { "  " } else { " " });
            }
            text += " ";
            text.extend(
                line.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' }),
            );
            text.push('\n');
        }
        ui.label(RichText::new(text).monospace().size(11.0));
    }
}

/// Mode name of CPSR bits 0-4.
pub const fn cpu_mode(cpsr: u32) -> &'static str {
    match cpsr & 0x1F {
        0x10 => "User",
        0x11 => "FIQ",
        0x12 => "IRQ",
        0x13 => "Supervisor",
        0x17 => "Abort",
        0x1B => "Undefined",
        0x1F => "System",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vram_mappings_follow_gbatek() {
        assert_eq!(vram_mapping(0, 0x00), "disabled");
        assert_eq!(vram_mapping(0, 0x80), "LCDC 06800000");
        assert_eq!(vram_mapping(0, 0x81 | (1 << 3)), "2D A BG 06020000");
        assert_eq!(vram_mapping(2, 0x84), "2D B BG 06200000");
        assert_eq!(vram_mapping(3, 0x82), "ARM7 06000000");
        assert_eq!(vram_mapping(8, 0x82), "2D B OBJ 06600000");
    }
}
