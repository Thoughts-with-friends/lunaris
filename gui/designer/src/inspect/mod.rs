//! The properties panel of the selected block or net (Vivado's "Block
//! Properties"): description, live registers with bit fields, and the
//! block's own view (audio, VRAM, memory, screen, CPU, FPGA).
//!
//! ```text
//!  selected block --> [Registers] [Audio | VRAM | Memory | Screen | FPGA]
//!  selected net   --> both ends, IF/IE of interrupt lines, end registers
//! ```

pub mod audio;
pub mod fpga;
pub mod regs;
pub mod views;

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Color32, RichText};

use crate::{
    diagram::hw::{self, NetDef},
    emu::Emu,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Registers,
    Special,
}

pub struct Inspector {
    pub catalog: regs::Catalog,
    tab: Tab,
    expanded: HashSet<String>,
    /// (block, register index) -> (value, time it last changed).
    history: HashMap<(&'static str, usize), (u32, f64)>,
    pub audio: audio::AudioView,
    vram: views::VramView,
    memory: views::MemoryView,
    fpga: fpga::FpgaView,
    last_block: Option<&'static str>,
}

/// The block-specific tab of `id`, if it has one.
const fn special_tab(id: &str) -> Option<&'static str> {
    match id.as_bytes() {
        b"speaker" | b"spu" => Some("Audio"),
        b"vram" => Some("VRAM"),
        b"main_ram" | b"wram" | b"wram7" => Some("Memory"),
        b"lcd_top" | b"lcd_bottom" | b"eng2d_a" | b"eng2d_b" | b"tsc" => Some("Screen"),
        b"slot2" => Some("FPGA"),
        b"arm9" | b"arm7" => Some("CPU"),
        _ => None,
    }
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            catalog: regs::Catalog::new(),
            tab: Tab::Registers,
            expanded: HashSet::new(),
            history: HashMap::new(),
            audio: audio::AudioView::new(),
            vram: views::VramView::new(),
            memory: views::MemoryView::new(),
            fpga: fpga::FpgaView::new(),
            last_block: None,
        }
    }

    pub fn show_block(&mut self, ui: &mut egui::Ui, id: &'static str, emu: &mut Emu, time: f64) {
        let Some(def) = hw::block_def(id) else { return };
        if self.last_block != Some(id) {
            self.last_block = Some(id);
            // Blocks with a view of their own open on it.
            self.tab = if special_tab(id).is_some() { Tab::Special } else { Tab::Registers };
        }
        ui.heading(def.title);
        ui.weak(def.subtitle);
        ui.label(def.description);
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Registers, "Registers");
            if let Some(name) = special_tab(id) {
                ui.selectable_value(&mut self.tab, Tab::Special, name);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match self.tab {
            Tab::Registers => self.registers(ui, id, emu, time),
            Tab::Special => self.special(ui, id, emu),
        });
    }

    fn special(&mut self, ui: &mut egui::Ui, id: &'static str, emu: &mut Emu) {
        match id {
            "speaker" | "spu" => {
                let frames = emu.frames;
                self.audio.show(ui, emu.nds.audio_tap(), emu.nds.emulator(), frames);
            }
            "vram" => self.vram.show(ui, &emu.nds),
            "main_ram" => self.memory.show(ui, &emu.nds.emulator().main_ram, 0x0200_0000),
            "wram" => self.memory.show(ui, &emu.nds.emulator().shared_wram, 0x0300_0000),
            "wram7" => self.memory.show(ui, &emu.nds.emulator().arm7_wram, 0x0380_0000),
            "slot2" => self.fpga.show(ui, emu),
            "arm9" | "arm7" => cpu_view(ui, emu, id == "arm9"),
            _ => screen_view(ui, emu, id),
        }
    }

    fn registers(&mut self, ui: &mut egui::Ui, id: &'static str, emu: &Emu, time: f64) {
        let regs = self.catalog.registers(id);
        if regs.is_empty() {
            ui.weak("No CPU-visible registers.");
            return;
        }
        let e = emu.nds.emulator();
        egui::Grid::new(("regs", id)).striped(true).num_columns(3).show(ui, |ui| {
            ui.strong("address");
            ui.strong("register");
            ui.strong("value");
            ui.end_row();
            for (i, reg) in regs.iter().enumerate() {
                let value = (reg.read)(e);
                let entry = self.history.entry((id, i)).or_insert((value, f64::NEG_INFINITY));
                if entry.0 != value {
                    *entry = (value, time);
                }
                // Recently changed values glow and fade over a second.
                let heat = (1.0 - (time - entry.1) as f32).clamp(0.0, 1.0);
                let color = Color32::from_rgb(255, 210, 60)
                    .lerp_to_gamma(ui.visuals().text_color(), 1.0 - heat);

                ui.monospace(if reg.addr == 0 {
                    "(core)".to_owned()
                } else {
                    format!("{:08X}", reg.addr)
                });
                let open = self.expanded.contains(&reg.name);
                let label = format!(
                    "{} {}",
                    if reg.fields.is_empty() {
                        " "
                    } else if open {
                        "v"
                    } else {
                        ">"
                    },
                    reg.name
                );
                if ui.selectable_label(open, label).clicked() && !reg.fields.is_empty() {
                    if !self.expanded.remove(&reg.name) {
                        self.expanded.insert(reg.name.clone());
                    }
                }
                let digits = usize::from(reg.bits / 4);
                ui.label(RichText::new(format!("{value:0digits$X}")).monospace().color(color))
                    .on_hover_text(format!(
                        "{value} / {value:#0w$b}",
                        w = usize::from(reg.bits) + 2
                    ));
                ui.end_row();
                if open {
                    for field in reg.fields {
                        let bits = if field.lo == field.hi {
                            format!("[{}]", field.lo)
                        } else {
                            format!("[{}:{}]", field.hi, field.lo)
                        };
                        ui.label("");
                        ui.weak(format!("   {bits} {}", field.name));
                        ui.monospace(format!("{:X}", field.extract(value)));
                        ui.end_row();
                    }
                }
            }
        });
    }

    pub fn show_net(&mut self, ui: &mut egui::Ui, net: &NetDef, emu: &Emu, time: f64) {
        ui.heading(net.name);
        ui.label(format!("{:?} net", net.kind));
        ui.monospace(format!("{}.{}  ->  {}.{}", net.from.0, net.from.1, net.to.0, net.to.1));
        if let Some((raised, pending)) = crate::diagram::canvas::irq_state(emu.nds.emulator(), net)
        {
            let (text, color) = match (raised, pending) {
                (true, _) => ("asserted (IF & IE)", Color32::from_rgb(230, 60, 60)),
                (false, true) => ("pending in IF, masked by IE", Color32::from_rgb(240, 170, 60)),
                _ => ("idle", Color32::GRAY),
            };
            ui.colored_label(color, text);
        }
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for end in [net.from.0, net.to.0] {
                let Some(def) = hw::block_def(end) else { continue };
                egui::CollapsingHeader::new(def.title).id_salt(end).default_open(true).show(
                    ui,
                    |ui| {
                        self.registers(ui, def.id, emu, time);
                    },
                );
            }
        });
    }
}

fn cpu_view(ui: &mut egui::Ui, emu: &Emu, arm9: bool) {
    let e = emu.nds.emulator();
    let cpu = if arm9 { &e.arm9 } else { &e.arm7 };
    let cpsr = cpu.cpsr.get();
    ui.label(
        RichText::new(format!(
            "mode {}  {}  {}{}",
            views::cpu_mode(cpsr),
            if cpsr & 0x20 != 0 { "Thumb" } else { "ARM" },
            if cpu.halted { "HALTED  " } else { "" },
            if cpsr & 0x80 != 0 { "IRQ off" } else { "IRQ on" },
        ))
        .strong(),
    );
    let flags: String = [(31, 'N'), (30, 'Z'), (29, 'C'), (28, 'V')]
        .iter()
        .map(|&(bit, c)| if cpsr & (1 << bit) != 0 { c } else { '-' })
        .collect();
    ui.monospace(format!("flags {flags}   CPSR {cpsr:08X}"));
    egui::Grid::new("cpu_regs").striped(true).num_columns(4).show(ui, |ui| {
        for row in 0..8 {
            for col in 0..2 {
                let i = row + col * 8;
                ui.monospace(match i {
                    13 => "SP ".to_owned(),
                    14 => "LR ".to_owned(),
                    15 => "PC ".to_owned(),
                    _ => format!("R{i:<2}"),
                });
                ui.monospace(format!("{:08X}", cpu.regs[i]));
            }
            ui.end_row();
        }
    });
}

fn screen_view(ui: &mut egui::Ui, emu: &Emu, id: &str) {
    let e = emu.nds.emulator();
    let a_on_top = e.gpu.get_powcnt1() & 0x8000 != 0;
    let (screen, note) = match id {
        "lcd_top" => (0, if a_on_top { "showing engine A" } else { "showing engine B" }),
        "eng2d_a" => (
            usize::from(!a_on_top),
            if a_on_top { "output on the top LCD" } else { "output on the bottom LCD" },
        ),
        "eng2d_b" => (
            usize::from(a_on_top),
            if a_on_top { "output on the bottom LCD" } else { "output on the top LCD" },
        ),
        _ => (1, if a_on_top { "showing engine B" } else { "showing engine A" }),
    };
    ui.label(note);
    let width = ui.available_width().min(512.0);
    ui.add(egui::Image::new((emu.screens[screen].id(), egui::vec2(width, width * 0.75))));
    ui.monospace(format!("VCOUNT {:3}   POWCNT1 {:04X}", e.gpu.get_vcount(), e.gpu.get_powcnt1()));
}
