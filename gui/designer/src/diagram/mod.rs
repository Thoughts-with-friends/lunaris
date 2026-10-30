//! The block design: catalogue ([`hw`]), canvas ([`canvas`]) and the state
//! they share — where each block sits, what is selected, which blocks are
//! busy right now.
//!
//! ```text
//!  every repaint: update_activity(emu)
//!     per block: hash of its live register values (inspect::regs)
//!     changed since last repaint -> activity = 1.0, then decays
//!     -> nets touching an active block animate on the canvas
//!  layout: block positions <-> instances/instance1/designer_layout.json
//! ```

pub mod canvas;
pub mod hw;

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
};

use ds_emu::Emulator;
use eframe::egui::{Pos2, Vec2, pos2, vec2};

use crate::inspect::regs::Catalog;

/// A block's ports, derived from the nets that name it.
#[derive(Debug, Default, Clone)]
pub struct Ports {
    pub inputs: Vec<(&'static str, hw::NetKind)>,
    pub outputs: Vec<(&'static str, hw::NetKind)>,
}

fn layout_path() -> PathBuf {
    lunaris_gui_common::config::Config::instance_dir(0).join("designer_layout.json")
}

pub struct Diagram {
    pub pan: Vec2,
    pub zoom: f32,
    pub pos: HashMap<&'static str, Pos2>,
    pub ports: HashMap<&'static str, Ports>,
    pub selected: Option<&'static str>,
    /// Index into [`hw::NETS`].
    pub selected_net: Option<usize>,
    dragging: Option<&'static str>,
    /// 0..=1 per block, 1 = registers changed this repaint.
    pub activity: HashMap<&'static str, f32>,
    signatures: HashMap<&'static str, u64>,
    /// Fit the view to the blocks at the next repaint.
    pub fit_pending: bool,
    /// Keep fitting when the canvas is resized, until the user pans/zooms.
    pub auto_fit: bool,
    pub last_size: Vec2,
}

impl Diagram {
    pub fn new() -> Self {
        let mut ports: HashMap<&'static str, Ports> = HashMap::new();
        for net in hw::NETS {
            let out = &mut ports.entry(net.from.0).or_default().outputs;
            if !out.iter().any(|(p, _)| *p == net.from.1) {
                out.push((net.from.1, net.kind));
            }
            let inp = &mut ports.entry(net.to.0).or_default().inputs;
            if !inp.iter().any(|(p, _)| *p == net.to.1) {
                inp.push((net.to.1, net.kind));
            }
        }
        let mut diagram = Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            pos: HashMap::new(),
            ports,
            selected: None,
            selected_net: None,
            dragging: None,
            activity: HashMap::new(),
            signatures: HashMap::new(),
            fit_pending: true,
            auto_fit: true,
            last_size: Vec2::ZERO,
        };
        diagram.reset_layout();
        diagram.load_layout();
        diagram
    }

    /// The catalogue's layout, with each column pushed apart so no block
    /// overlaps the one above it (heights depend on the port count).
    pub fn reset_layout(&mut self) {
        let mut blocks: Vec<&hw::BlockDef> = hw::BLOCKS.iter().collect();
        blocks.sort_by(|a, b| a.pos.0.total_cmp(&b.pos.0).then(a.pos.1.total_cmp(&b.pos.1)));
        let mut column = f32::NAN;
        let mut bottom = f32::NEG_INFINITY;
        for b in blocks {
            if b.pos.0 != column {
                column = b.pos.0;
                bottom = f32::NEG_INFINITY;
            }
            let y = b.pos.1.max(bottom + 30.0);
            bottom = y + self.block_size(b.id).y;
            self.pos.insert(b.id, pos2(b.pos.0, y));
        }
        self.fit_pending = true;
        self.auto_fit = true;
    }

    fn load_layout(&mut self) {
        let Ok(text) = std::fs::read_to_string(layout_path()) else { return };
        let Ok(saved) = serde_json::from_str::<HashMap<String, (f32, f32)>>(&text) else { return };
        for b in hw::BLOCKS {
            if let Some(&(x, y)) = saved.get(b.id) {
                self.pos.insert(b.id, pos2(x, y));
            }
        }
    }

    pub fn save_layout(&self) {
        let saved: HashMap<&str, (f32, f32)> =
            self.pos.iter().map(|(k, p)| (*k, (p.x, p.y))).collect();
        if let Ok(json) = serde_json::to_string_pretty(&saved) {
            let _ = std::fs::write(layout_path(), json);
        }
    }

    /// Block size in canvas units.
    pub fn block_size(&self, id: &str) -> Vec2 {
        let rows = self.ports.get(id).map_or(0, |p| p.inputs.len().max(p.outputs.len()));
        vec2(210.0, canvas::HEADER + rows as f32 * canvas::ROW + 10.0)
    }

    /// Marks blocks whose registers changed since the last call and lets
    /// older activity fade.
    pub fn update_activity(&mut self, emu: &Emulator, catalog: &Catalog, dt: f32) {
        for b in hw::BLOCKS {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for reg in catalog.registers(b.id) {
                (reg.read)(emu).hash(&mut hasher);
            }
            let sig = hasher.finish();
            let level = self.activity.entry(b.id).or_insert(0.0);
            if self.signatures.insert(b.id, sig).is_some_and(|old| old != sig) {
                *level = 1.0;
            } else {
                *level = (*level - dt * 1.5).max(0.0);
            }
        }
    }

    pub fn select(&mut self, id: &'static str) {
        self.selected = Some(id);
        self.selected_net = None;
    }

    /// Centres `id` in a view of `size`.
    pub fn focus(&mut self, id: &str, size: Vec2) {
        if let Some(p) = self.pos.get(id) {
            let center = *p + self.block_size(id) / 2.0;
            self.pan = size / 2.0 - center.to_vec2() * self.zoom;
            self.fit_pending = false;
            self.auto_fit = false;
        }
    }
}
