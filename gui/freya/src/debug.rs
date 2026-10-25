//! The Debug panel: the egui GUI's Palettes / Maps / Tiles / VRAM windows
//! and Performance stats, as a side panel next to the screens (not a modal
//! dialog, so the game keeps taking input).
//!
//! ```text
//!  controls --DebugView--> Shared::set_debug_view
//!  emulator thread, every 6 frames: Core::debug_image -> mailbox
//!  pump (50 ms) --Shared::debug_image_since--> ImageHandle -> image()
//!  Stats: Shared::frame_times (last 20 s) -> bars
//! ```
use std::time::Duration;

use async_io::Timer;
use bytes::Bytes;
use freya::{elements::image::ImageHandle, engine::prelude::AlphaType, prelude::*};
use futures_lite::StreamExt;

use crate::{
    backend::{BACKEND, DebugView},
    dialogs::section,
    menu::Act,
    state::Ctx,
    theme::{self, ACCENT, TEXT, TEXT_DIM, TEXT_FAINT},
};

/// Width of the panel.
pub const PANEL_WIDTH: f32 = 340.0;
/// How often the panel looks for a new picture.
const POLL: Duration = Duration::from_millis(50);
/// Stats bars drawn (the newest frames).
const BARS: usize = 120;

/// The view the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugTab {
    Palettes,
    Maps,
    Tiles,
    Vram,
    Stats,
}

impl DebugTab {
    pub const ALL: [Self; 5] = [Self::Palettes, Self::Maps, Self::Tiles, Self::Vram, Self::Stats];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Palettes => "Palettes",
            Self::Maps => "Maps",
            Self::Tiles => "Tiles",
            Self::Vram => "VRAM",
            Self::Stats => "Performance stats",
        }
    }

    /// The name on the panel's view switcher.
    const fn short(self) -> &'static str {
        match self {
            Self::Stats => "Stats",
            _ => self.label(),
        }
    }

    /// Whether this build's core can show it (Stats works on every core).
    pub const fn available(self) -> bool {
        matches!(self, Self::Stats) || BACKEND.has_debug()
    }
}

/// The settings of every view, kept while switching between them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Views {
    palettes: DebugView,
    maps: DebugView,
    tiles: DebugView,
    vram: DebugView,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            palettes: DebugView::Palettes {
                engine_b: false,
                obj: false,
                extended: false,
                slot: 0,
                palette: 0,
            },
            maps: DebugView::Maps { engine_b: false, bg: 0 },
            tiles: DebugView::Tiles {
                engine_b: false,
                obj: false,
                extended: false,
                bitmap: false,
                bpp8: false,
                slot: 0,
                palette: 0,
                offset: 0,
            },
            vram: DebugView::Vram { bank: 0, ignore_alpha: false },
        }
    }
}

impl Views {
    const fn view(&self, tab: DebugTab) -> Option<DebugView> {
        Some(match tab {
            DebugTab::Palettes => self.palettes,
            DebugTab::Maps => self.maps,
            DebugTab::Tiles => self.tiles,
            DebugTab::Vram => self.vram,
            DebugTab::Stats => return None,
        })
    }

    const fn get(&mut self, tab: DebugTab) -> Option<&mut DebugView> {
        Some(match tab {
            DebugTab::Palettes => &mut self.palettes,
            DebugTab::Maps => &mut self.maps,
            DebugTab::Tiles => &mut self.tiles,
            DebugTab::Vram => &mut self.vram,
            DebugTab::Stats => return None,
        })
    }
}

/// One row of choices; `on` gets the picked index.
fn pills(
    label: &str,
    options: &[String],
    selected: usize,
    on: impl Fn(usize) + Clone + 'static,
) -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .padding((3., 0.))
        .child(
            rect()
                .width(Size::px(84.))
                .font_size(12.)
                .color(theme::rgb(TEXT_DIM))
                .child(label.to_owned()),
        )
        .child(
            rect()
                .horizontal()
                .width(Size::px(240.))
                .content(Content::wrap())
                .spacing(4.)
                .children(options.iter().enumerate().map(|(i, text)| {
                    let on = on.clone();
                    let active = i == selected;
                    rect()
                        .key(i)
                        .padding((2., 8.))
                        .margin((0., 0., 4., 0.))
                        .corner_radius(8.)
                        .font_size(12.)
                        .background(if active { theme::rgba(ACCENT, 90) } else { theme::surface() })
                        .color(theme::rgb(if active { TEXT } else { TEXT_DIM }))
                        .on_press(move |_| on(i))
                        .child(text.clone())
                        .into_element()
                })),
        )
        .into_element()
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn numbers(range: std::ops::RangeInclusive<u8>) -> Vec<String> {
    range.map(|n| n.to_string()).collect()
}

/// Changes one field of a view to the picked index.
type Apply = fn(&mut DebugView, usize);

/// The controls of `view`; `edit` changes the stored view.
fn controls(
    view: DebugView,
    edit: impl Fn(&dyn Fn(&mut DebugView)) + Clone + 'static,
) -> Vec<Element> {
    use DebugView as V;

    let row = |label: &str, options: Vec<String>, selected: usize, apply: Apply| {
        let edit = edit.clone();
        pills(label, &options, selected, move |i| edit(&|v| apply(v, i)))
    };
    let ab = || names(&["A", "B"]);
    let kinds = || names(&["BG", "OBJ"]);
    let onoff = || names(&["Off", "On"]);
    let mut out = Vec::new();
    match view {
        V::Palettes { engine_b, obj, extended, slot, palette } => {
            out.push(row("Engine", ab(), engine_b.into(), |v, i| {
                if let V::Palettes { engine_b, .. } = v {
                    *engine_b = i == 1;
                }
            }));
            out.push(row("Type", kinds(), obj.into(), |v, i| {
                if let V::Palettes { obj, .. } = v {
                    *obj = i == 1;
                }
            }));
            out.push(row("Extended", onoff(), extended.into(), |v, i| {
                if let V::Palettes { extended, .. } = v {
                    *extended = i == 1;
                }
            }));
            if extended {
                if !obj {
                    out.push(row("Slot", numbers(0..=3), slot.into(), |v, i| {
                        if let V::Palettes { slot, .. } = v {
                            *slot = i as u8;
                        }
                    }));
                }
                out.push(row("Palette", numbers(0..=15), palette.into(), |v, i| {
                    if let V::Palettes { palette, .. } = v {
                        *palette = i as u8;
                    }
                }));
            }
        }
        V::Maps { engine_b, bg } => {
            out.push(row("Engine", ab(), engine_b.into(), |v, i| {
                if let V::Maps { engine_b, .. } = v {
                    *engine_b = i == 1;
                }
            }));
            out.push(row("BG", numbers(0..=3), bg.into(), |v, i| {
                if let V::Maps { bg, .. } = v {
                    *bg = i as u8;
                }
            }));
        }
        V::Tiles { engine_b, obj, extended, bitmap, bpp8, slot, palette, offset } => {
            out.push(row("Engine", ab(), engine_b.into(), |v, i| {
                if let V::Tiles { engine_b, .. } = v {
                    *engine_b = i == 1;
                }
            }));
            out.push(row("Type", kinds(), obj.into(), |v, i| {
                if let V::Tiles { obj, offset, .. } = v {
                    *obj = i == 1;
                    *offset = 0;
                }
            }));
            if !extended && !bpp8 {
                out.push(row("Bitmap", onoff(), bitmap.into(), |v, i| {
                    if let V::Tiles { bitmap, .. } = v {
                        *bitmap = i == 1;
                    }
                }));
            }
            if !bitmap {
                out.push(row("Ext. palettes", onoff(), extended.into(), |v, i| {
                    if let V::Tiles { extended, .. } = v {
                        *extended = i == 1;
                    }
                }));
                if !extended {
                    out.push(row("256 colors", onoff(), bpp8.into(), |v, i| {
                        if let V::Tiles { bpp8, .. } = v {
                            *bpp8 = i == 1;
                        }
                    }));
                } else if !obj {
                    out.push(row("Slot", numbers(0..=3), slot.into(), |v, i| {
                        if let V::Tiles { slot, .. } = v {
                            *slot = i as u8;
                        }
                    }));
                }
                if extended || !bpp8 {
                    out.push(row("Palette", numbers(0..=15), palette.into(), |v, i| {
                        if let V::Tiles { palette, .. } = v {
                            *palette = i as u8;
                        }
                    }));
                }
            }
            // Engine A only, as the egui window (BG 0-3, OBJ 0-1).
            if !engine_b {
                let last = if obj { 1 } else { 3 };
                out.push(row("Offset", numbers(0..=last), offset.into(), |v, i| {
                    if let V::Tiles { offset, .. } = v {
                        *offset = i as u8;
                    }
                }));
            }
        }
        V::Vram { bank, ignore_alpha } => {
            let banks = "ABCDEFGHI".chars().map(String::from).collect();
            out.push(row("Bank", banks, bank.into(), |v, i| {
                if let V::Vram { bank, .. } = v {
                    *bank = i as u8;
                }
            }));
            out.push(row("Ignore alpha", onoff(), ignore_alpha.into(), |v, i| {
                if let V::Vram { ignore_alpha, .. } = v {
                    *ignore_alpha = i == 1;
                }
            }));
        }
    }
    out
}

/// The latest picture of the open view.
#[derive(Clone, PartialEq)]
struct Picture {
    image: ImageHandle,
    width: f32,
    height: f32,
}

#[derive(PartialEq)]
pub struct DebugPanel;

impl Component for DebugPanel {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let tab = (*ctx.debug.read()).unwrap_or(DebugTab::Stats);
        let views = use_state(Views::default);
        let mut picture = use_state(|| None::<Picture>);
        let mut times = use_state(Vec::<f32>::new);
        let zoom = use_state(|| 1_u8);

        let view = views.read().view(tab);
        ctx.front.shared.set_debug_view(view.filter(|_| tab.available()));

        let pump_ctx = ctx.clone();
        use_hook(move || {
            spawn(async move {
                let mut last = 0;
                let mut tick = Timer::interval(POLL);
                let mut n = 0_u32;
                loop {
                    tick.next().await;
                    n += 1;
                    let shared = &pump_ctx.front.shared;
                    if let Some((seq, (rgba, w, h))) = shared.debug_image_since(last) {
                        last = seq;
                        if let Some(image) = ImageHandle::from_rgba(
                            w as u32,
                            h as u32,
                            Bytes::from(rgba),
                            AlphaType::Unpremul,
                        ) {
                            picture.set(Some(Picture { image, width: w as f32, height: h as f32 }));
                        }
                    }
                    if n.is_multiple_of(5) && *pump_ctx.debug.peek() == Some(DebugTab::Stats) {
                        times.set(shared.frame_times());
                    }
                }
            });
        });

        let close_ctx = ctx.clone();
        let header = rect()
            .horizontal()
            .content(Content::Flex)
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .child(
                rect()
                    .width(Size::flex(1.))
                    .font_size(14.)
                    .font_weight(FontWeight::BOLD)
                    .child(format!("Debug · {}", tab.label())),
            )
            .child(
                rect()
                    .color(theme::rgb(TEXT_DIM))
                    .on_press(move |_| crate::actions::run(&close_ctx, Act::Debug(tab)))
                    .child("✕"),
            );

        let tabs = pills(
            "View",
            &DebugTab::ALL.iter().map(|t| t.short().to_owned()).collect::<Vec<_>>(),
            DebugTab::ALL.iter().position(|t| *t == tab).unwrap_or(0),
            {
                move |i| {
                    let mut debug = ctx.debug;
                    debug.set(Some(DebugTab::ALL[i]));
                    ctx.refresh();
                }
            },
        );

        let body: Vec<Element> = if !tab.available() {
            vec![
                rect()
                    .padding((12., 0.))
                    .font_size(12.)
                    .color(theme::rgb(TEXT_FAINT))
                    .child(format!("The {BACKEND} core has no VRAM views (Lunaris only)."))
                    .into_element(),
            ]
        } else if tab == DebugTab::Stats {
            stats(&times.read())
        } else {
            let edit = move |change: &dyn Fn(&mut DebugView)| {
                let mut views = views;
                if let Some(view) = views.write().get(tab) {
                    change(view);
                }
            };
            let mut out = view.map(|v| controls(v, edit)).unwrap_or_default();
            let z = zoom();
            out.push(pills("Zoom", &names(&["1x", "2x", "3x"]), usize::from(z - 1), move |i| {
                let mut zoom = zoom;
                zoom.set(i as u8 + 1);
            }));
            if let Some(p) = picture.read().clone() {
                out.push(
                    rect()
                        .margin((8., 0., 0., 0.))
                        .child(
                            image(p.image)
                                .width(Size::px(p.width * f32::from(z)))
                                .height(Size::px(p.height * f32::from(z)))
                                .aspect_ratio(AspectRatio::None)
                                .sampling_mode(SamplingMode::Nearest),
                        )
                        .into_element(),
                );
                out.push(
                    rect()
                        .font_size(11.)
                        .color(theme::rgb(TEXT_FAINT))
                        .child(format!("{}×{}", p.width, p.height))
                        .into_element(),
                );
            }
            out
        };

        rect()
            .width(Size::px(PANEL_WIDTH))
            .height(Size::fill())
            .padding(12.)
            .background(theme::panel())
            .border(Border::new().width(1.).fill(theme::hairline()))
            .corner_radius(10.)
            .child(header)
            .child(section("view"))
            .child(tabs)
            .child(
                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::fill())
                    .child(rect().width(Size::fill()).children(body)),
            )
    }
}

/// Frame-time bars (16.7 ms = half height) and the average.
fn stats(times: &[f32]) -> Vec<Element> {
    if times.is_empty() {
        return vec![
            rect()
                .font_size(12.)
                .color(theme::rgb(TEXT_FAINT))
                .child("No frames yet")
                .into_element(),
        ];
    }
    let avg = times.iter().sum::<f32>() / times.len() as f32;
    let worst = times.iter().copied().fold(0., f32::max);
    let recent = &times[times.len().saturating_sub(BARS)..];
    let height = 120.0_f32;
    let bars = rect()
        .horizontal()
        .main_align(Alignment::End)
        .cross_align(Alignment::End)
        .width(Size::px(BARS as f32 * 2.5))
        .height(Size::px(height))
        .background(theme::surface())
        .children(recent.iter().enumerate().map(|(i, t)| {
            let h = (t / (2.0 / 60.0) * height).clamp(1., height);
            let slow = *t > 1.5 / 60.0;
            rect()
                .key(i)
                .width(Size::px(2.5))
                .height(Size::px(h))
                .background(theme::rgba(if slow { theme::WARNING } else { ACCENT }, 200))
                .into_element()
        }));
    vec![
        section("frame times"),
        bars.into_element(),
        rect()
            .margin((8., 0., 0., 0.))
            .font_size(12.)
            .color(theme::rgb(TEXT_DIM))
            .child(format!(
                "Average {:.2} ms ({:.1} fps) · worst {:.2} ms · {} frames",
                avg * 1000.,
                1. / avg.max(1e-6),
                worst * 1000.,
                times.len()
            ))
            .into_element(),
    ]
}
