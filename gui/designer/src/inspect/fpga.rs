//! The Slot-2 FPGA: insert/remove it, load blocks (JSON specs, or Verilog /
//! SystemVerilog run by the built-in simulator), watch and poke their
//! registers, operate the samples (toggle switch, RISC-V), read the bus
//! log, and see each block's source.
//!
//! ```text
//!  [x] FPGA in Slot-2      Add: timer | toggle switch | RISC-V | Load...
//!  block fpga_timer @ 08000000..08000100  spec model  irq=0
//!    +0x0000 ID       const   46504741
//!    +0x0004 CTRL     rw      00000001   [write]
//!    ...
//!  Source: (Verilog) (SystemVerilog) (Rust)   Export all...
//!  Bus log: R/W  address  width  value  register
//! ```

use ds_emu::slot2::{Slot2Device, Width};
use ds_fpga::{BlockSpec, FpgaSlot, codegen::Language};
use eframe::egui::{self, Color32, RichText};

use crate::emu::Emu;

pub struct FpgaView {
    lang: Language,
    block: usize,
    poke: String,
    status: String,
    /// RISC-V sample: value for MAILBOX_IN, and n of the Fibonacci demo.
    mailbox_in: String,
    fib_n: i32,
}

impl FpgaView {
    pub const fn new() -> Self {
        Self {
            lang: Language::Verilog,
            block: 0,
            poke: String::new(),
            status: String::new(),
            mailbox_in: String::new(),
            fib_n: 20,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, emu: &mut Emu) {
        let mut inserted = emu.fpga().is_some();
        if ui.checkbox(&mut inserted, "FPGA cartridge inserted in Slot-2").changed() {
            emu.set_fpga_inserted(inserted);
        }
        let owner = if emu.nds.emulator().ex_mem_cnt & 0x80 != 0 { "ARM7" } else { "ARM9" };
        ui.weak(format!("Slot-2 bus owner (EXMEMCNT bit 7): {owner}. IRQ -> ARM9 IF bit 13."));
        let Some(fpga) = emu.fpga() else {
            ui.label("Insert the cartridge to place FPGA blocks on the 08000000h bus.");
            return;
        };

        ui.horizontal_wrapped(|ui| {
            ui.label("Add:");
            if ui.button("Timer (JSON spec)").clicked() {
                self.add(fpga, ds_fpga::example_timer());
            }
            if ui.button("Toggle switch (Verilog)").clicked() {
                self.add(fpga, ds_fpga::samples::toggle_switch());
            }
            if ui.button("RISC-V RV32I (Verilog)").clicked() {
                self.add(fpga, ds_fpga::samples::riscv_rv32i());
            }
            if ui.button("Load block (.json / .v / .sv)...").clicked() {
                self.load_file(fpga);
            }
        });
        if !self.status.is_empty() {
            ui.colored_label(Color32::from_rgb(230, 160, 60), &self.status);
        }
        if fpga.blocks().is_empty() {
            ui.label("No blocks: the slot reads as open bus.");
            return;
        }
        self.block = self.block.min(fpga.blocks().len() - 1);

        ui.horizontal_wrapped(|ui| {
            for (i, b) in fpga.blocks().iter().enumerate() {
                if ui.selectable_label(self.block == i, &b.spec.name).clicked() {
                    self.block = i;
                }
            }
        });
        ui.separator();
        self.device_panel(ui, fpga);
        self.block_view(ui, fpga);
        ui.separator();
        self.source_view(ui, fpga);
        ui.separator();
        bus_log(ui, fpga);
    }

    fn load_file(&mut self, fpga: &mut FpgaSlot) {
        let dialog = rfd::AsyncFileDialog::new()
            .add_filter("FPGA block", &["json", "v", "sv"])
            .set_directory(concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/fpga/hdl"));
        let Some(file) = pollster::block_on(dialog.pick_file()) else { return };
        let path = file.path();
        let loaded = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|text| {
            match path.extension().and_then(|e| e.to_str()) {
                Some("json") => BlockSpec::from_json(&text).map_err(|e| e.to_string()),
                _ => BlockSpec::from_verilog(&text).map_err(|e| e.to_string()),
            }
        });
        match loaded {
            Ok(spec) => self.add(fpga, spec),
            Err(e) => self.status = format!("cannot load {}: {e}", path.display()),
        }
    }

    /// Controls for blocks with a known register map: a switch + LED
    /// (SWITCH, LED) and a CPU (CTRL, IMEM, MAILBOX_OUT).
    fn device_panel(&mut self, ui: &mut egui::Ui, fpga: &mut FpgaSlot) {
        let Some(block) = fpga.blocks().get(self.block) else { return };
        let base = block.window().start;
        let reg = |name: &str| block.spec.register(name).map(|r| r.offset);
        let peek = |offset: u16| block.peek(offset);
        let mut writes: Vec<(u16, u32)> = Vec::new();

        if let (Some(switch), Some(led)) = (reg("SWITCH"), reg("LED")) {
            let on = peek(led) & 1 != 0;
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                let color = if on {
                    Color32::from_rgb(80, 240, 120)
                } else {
                    Color32::from_rgb(40, 60, 45)
                };
                if on {
                    ui.painter().circle_filled(rect.center(), 13.0, color.gamma_multiply(0.3));
                }
                ui.painter().circle(
                    rect.center(),
                    9.0,
                    color,
                    egui::Stroke::new(1.5_f32, Color32::from_gray(30)),
                );
                ui.label(RichText::new(if on { "LED ON" } else { "LED OFF" }).strong());
                if ui.button("Press switch").on_hover_text("writes 1 to SWITCH").clicked() {
                    writes.push((switch, 1));
                }
                if let Some(presses) = reg("PRESSES") {
                    ui.weak(format!("{} presses", peek(presses)));
                }
            });
            ui.separator();
        }

        if let (Some(ctrl), Some(imem), Some(out)) = (reg("CTRL"), reg("IMEM"), reg("MAILBOX_OUT"))
        {
            let status = reg("STATUS").map_or(0, peek);
            let state = match (status & 1 != 0, status & 2 != 0, status & 4 != 0) {
                (_, true, true) => ("halted: illegal instruction", Color32::from_rgb(230, 80, 80)),
                (_, true, false) => ("halted (ECALL)", Color32::from_rgb(240, 170, 60)),
                (true, false, _) => ("running", Color32::from_rgb(90, 220, 120)),
                _ => ("stopped", Color32::GRAY),
            };
            ui.horizontal(|ui| {
                ui.colored_label(state.1, RichText::new(state.0).strong());
                ui.monospace(format!(
                    "PC {:08X}  retired {}",
                    reg("PC").map_or(0, peek),
                    reg("INSTRET").map_or(0, peek)
                ));
            });
            let mailbox = peek(out);
            ui.monospace(format!("MAILBOX_OUT = {mailbox} (0x{mailbox:08X})"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.fib_n).range(1..=40).prefix("n = "));
                if ui
                    .button("Load Fibonacci demo + run")
                    .on_hover_text("DMEM[i] = fib(i); MAILBOX_OUT = fib(n); ECALL")
                    .clicked()
                {
                    let program = ds_fpga::samples::fibonacci(self.fib_n);
                    writes.push((ctrl, 0));
                    writes
                        .extend(program.iter().enumerate().map(|(i, w)| (imem + 4 * i as u16, *w)));
                    if let Some(en) = reg("IRQ_EN") {
                        writes.push((en, 0b11));
                    }
                    writes.push((ctrl, 0b11));
                }
                if ui
                    .button("Load counter demo + run")
                    .on_hover_text("endless loop: MAILBOX_OUT = 1, 2, 3, ...")
                    .clicked()
                {
                    writes.push((ctrl, 0));
                    writes.extend(
                        ds_fpga::samples::counter()
                            .iter()
                            .enumerate()
                            .map(|(i, w)| (imem + 4 * i as u16, *w)),
                    );
                    writes.push((ctrl, 0b11));
                }
                if ui
                    .button("Load program...")
                    .on_hover_text(".bin (little-endian words) or .hex (one word per line)")
                    .clicked()
                {
                    match load_program() {
                        Ok(Some(words)) => {
                            writes.push((ctrl, 0));
                            writes.extend(
                                words
                                    .iter()
                                    .take(1024)
                                    .enumerate()
                                    .map(|(i, w)| (imem + 4 * i as u16, *w)),
                            );
                            self.status = format!("loaded {} words", words.len().min(1024));
                        }
                        Ok(None) => {}
                        Err(e) => self.status = e,
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    writes.push((ctrl, 0b01));
                }
                if ui.button("Stop").clicked() {
                    writes.push((ctrl, 0b00));
                }
                if ui.button("Reset + run").clicked() {
                    writes.push((ctrl, 0b11));
                }
                if let Some(inbox) = reg("MAILBOX_IN") {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mailbox_in)
                            .desired_width(70.0)
                            .hint_text("MAILBOX_IN"),
                    );
                    if ui.button("Send").clicked() {
                        writes.push((inbox, parse_word(&self.mailbox_in).unwrap_or(0)));
                    }
                }
            });
            ui.separator();
        }

        for (offset, value) in writes {
            fpga.write(base + u32::from(offset), value, Width::Word);
        }
    }

    fn add(&mut self, fpga: &mut FpgaSlot, mut spec: BlockSpec) {
        // Slide the block to the first free window instead of refusing it.
        let window = ds_emu::slot2::SLOT2_END - ds_emu::slot2::SLOT2_START;
        while spec.base + spec.size <= window {
            let base = spec.base;
            let taken = fpga
                .blocks()
                .iter()
                .any(|b| base < b.spec.base + b.spec.size && b.spec.base < base + spec.size);
            if !taken {
                break;
            }
            spec.base += spec.size;
        }
        let name = spec.name.clone();
        self.status = match fpga.add_spec(spec) {
            Ok(()) => {
                self.block = fpga.blocks().len() - 1;
                format!("added {name}")
            }
            Err(e) => e.to_string(),
        };
    }

    fn block_view(&mut self, ui: &mut egui::Ui, fpga: &mut FpgaSlot) {
        let index = self.block;
        let block = &fpga.blocks()[index];
        let window = block.window();
        ui.label(
            RichText::new(format!(
                "{}  @ {:08X}..{:08X}",
                block.spec.name, window.start, window.end
            ))
            .strong(),
        );
        ui.weak(format!(
            "{} | clock 33.51 MHz / {} | irq {}",
            block.backend,
            block.spec.clock_div,
            u8::from(block.irq)
        ));
        if !block.spec.description.is_empty() {
            ui.label(&block.spec.description);
        }
        let mut write: Option<(u32, u32)> = None;
        let mut remove = false;
        egui::Grid::new("fpga_regs").striped(true).num_columns(5).show(ui, |ui| {
            for h in ["offset", "name", "access", "value", ""] {
                ui.strong(h);
            }
            ui.end_row();
            for reg in &block.spec.registers {
                ui.monospace(format!("+{:04X}", reg.offset));
                ui.label(&reg.name).on_hover_text(&reg.description);
                ui.label(ds_fpga::codegen::access_label(&reg.access));
                ui.monospace(format!("{:08X}", block.peek(reg.offset)));
                if matches!(reg.access, ds_fpga::Access::Rw | ds_fpga::Access::W1c { .. }) {
                    if ui
                        .small_button("write")
                        .on_hover_text("write the value below through the bus")
                        .clicked()
                    {
                        let value =
                            u32::from_str_radix(self.poke.trim().trim_start_matches("0x"), 16)
                                .unwrap_or(0);
                        write = Some((window.start + u32::from(reg.offset), value));
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.label("value (hex)");
            ui.add(egui::TextEdit::singleline(&mut self.poke).desired_width(90.0));
            if ui.button("Remove block").clicked() {
                remove = true;
            }
        });
        if let Some((addr, value)) = write {
            fpga.write(addr, value, Width::Word);
        }
        if remove {
            fpga.remove(index);
        }
    }

    fn source_view(&mut self, ui: &mut egui::Ui, fpga: &FpgaSlot) {
        let Some(block) = fpga.blocks().get(self.block) else { return };
        if let Some(rtl) = &block.spec.rtl {
            ui.horizontal(|ui| {
                ui.label(RichText::new("RTL source").strong());
                ui.weak("(runs in the built-in Verilog simulator)");
                if ui.button("Save as...").clicked() {
                    let name = format!("{}.v", block.spec.name);
                    if let Some(file) = pollster::block_on(
                        rfd::AsyncFileDialog::new().set_file_name(name).save_file(),
                    ) {
                        self.status = match std::fs::write(file.path(), rtl) {
                            Ok(()) => format!("saved {}", file.path().display()),
                            Err(e) => format!("save failed: {e}"),
                        };
                    }
                }
            });
            let mut source = rtl.clone();
            show_code(ui, &mut source);
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Generated source").strong());
            for lang in Language::ALL {
                ui.selectable_value(&mut self.lang, lang, lang.label());
            }
            if ui.button("Export .v / .sv / .rs ...").clicked() {
                if let Some(dir) = pollster::block_on(rfd::AsyncFileDialog::new().pick_folder()) {
                    // One folder per project, as in core/fpga/hdl/<name>/.
                    let dir = dir.path().join(&block.spec.name);
                    self.status = match ds_fpga::codegen::write_all(&block.spec, &dir) {
                        Ok(paths) => format!("wrote {} files to {}", paths.len(), dir.display()),
                        Err(e) => format!("export failed: {e}"),
                    };
                }
            }
        });
        let mut source = ds_fpga::codegen::generate(&block.spec, self.lang);
        show_code(ui, &mut source);
    }
}

fn show_code(ui: &mut egui::Ui, source: &mut String) {
    egui::ScrollArea::both().id_salt("fpga_source").max_height(260.0).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(source)
                .code_editor()
                .desired_width(f32::INFINITY)
                .interactive(false),
        );
    });
}

fn parse_word(text: &str) -> Option<u32> {
    let t = text.trim();
    match t.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => t.parse().ok(),
    }
}

/// A RISC-V program picked by the user: `.hex` text (one hex word per
/// token) or raw little-endian `.bin`. `Ok(None)`: cancelled.
fn load_program() -> Result<Option<Vec<u32>>, String> {
    let dialog = rfd::AsyncFileDialog::new().add_filter("Program", &["bin", "hex", "txt"]);
    let Some(file) = pollster::block_on(dialog.pick_file()) else { return Ok(None) };
    let bytes = std::fs::read(file.path()).map_err(|e| e.to_string())?;
    let is_text = matches!(file.path().extension().and_then(|e| e.to_str()), Some("hex" | "txt"));
    let words = if is_text {
        String::from_utf8_lossy(&bytes)
            .split_whitespace()
            .filter(|t| !t.starts_with('@') && !t.starts_with("//"))
            .map(|t| {
                u32::from_str_radix(t.trim_start_matches("0x"), 16)
                    .map_err(|_| format!("bad word `{t}`"))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        bytes
            .chunks(4)
            .map(|c| c.iter().rev().fold(0, |acc, &b| (acc << 8) | u32::from(b)))
            .collect()
    };
    Ok(Some(words))
}

fn bus_log(ui: &mut egui::Ui, fpga: &FpgaSlot) {
    ui.label(RichText::new(format!("Bus log ({} accesses)", fpga.accesses)).strong());
    egui::ScrollArea::vertical().id_salt("fpga_log").max_height(180.0).stick_to_bottom(true).show(
        ui,
        |ui| {
            egui::Grid::new("fpga_log_grid").striped(true).num_columns(5).show(ui, |ui| {
                for ev in fpga.log() {
                    let block = &fpga.blocks().get(ev.block);
                    let reg = block.and_then(|b| {
                        let offset = (ev.addr - b.window().start) & !3;
                        b.spec
                            .registers
                            .iter()
                            .find(|r| u32::from(r.offset) == offset)
                            .map(|r| r.name.as_str())
                    });
                    let (rw, color) = if ev.write {
                        ("W", Color32::from_rgb(240, 150, 60))
                    } else {
                        ("R", Color32::from_rgb(100, 170, 255))
                    };
                    ui.colored_label(color, rw);
                    ui.monospace(format!("{:08X}", ev.addr));
                    ui.monospace(match ev.width {
                        Width::Byte => "8",
                        Width::Half => "16",
                        Width::Word => "32",
                    });
                    ui.monospace(format!("{:08X}", ev.value));
                    ui.label(reg.unwrap_or("-"));
                    ui.end_row();
                }
            });
        },
    );
}
