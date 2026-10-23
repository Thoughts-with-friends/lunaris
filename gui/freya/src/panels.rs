//! The welcome screen (no game running) and the status bar.
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use ds_common::{
    frontend::{file_name, rom_paths},
    menu::Action,
    rom_info::{ICON_SIZE, RomInfo},
};
use freya::{elements::image::ImageHandle, engine::prelude::AlphaType, prelude::*};

use crate::{
    actions::{self, STATUSBAR_HEIGHT},
    backend::BACKEND,
    state::Ctx,
    theme::{self, ACCENT, DANGER, SUCCESS, TEXT, TEXT_DIM, TEXT_FAINT, WARNING},
    titlebar::logo,
};

/// Most ROM cards shown.
const MAX_CARDS: usize = 12;

/// Banner title and icon of a ROM.
#[derive(Clone, PartialEq)]
struct Card {
    title: String,
    icon: Option<ImageHandle>,
}

thread_local! {
    /// ROM headers read once per path.
    static CARDS: RefCell<HashMap<PathBuf, Card>> = RefCell::new(HashMap::new());
}

fn card(path: &Path) -> Card {
    CARDS.with_borrow_mut(|cards| {
        cards
            .entry(path.to_owned())
            .or_insert_with(|| {
                let info = RomInfo::read(path).ok();
                let title = info
                    .as_ref()
                    .and_then(|i| i.title_ja.clone().or_else(|| i.title_en.clone()))
                    .map_or_else(
                        || file_name(path),
                        |t| t.lines().next().unwrap_or_default().to_owned(),
                    );
                let icon = info.filter(|i| !i.icon_rgba.is_empty()).and_then(|i| {
                    ImageHandle::from_rgba(
                        ICON_SIZE as u32,
                        ICON_SIZE as u32,
                        Bytes::from(i.icon_rgba),
                        AlphaType::Unpremul,
                    )
                });
                Card { title, icon }
            })
            .clone()
    })
}

/// A ROM's banner icon (a placeholder without one).
pub fn rom_icon(icon: Option<ImageHandle>, size: f32) -> Element {
    match icon {
        Some(icon) => rect()
            .width(Size::px(size))
            .height(Size::px(size))
            .corner_radius(8.)
            .overflow(Overflow::Clip)
            .child(
                image(icon)
                    .expanded()
                    .aspect_ratio(AspectRatio::None)
                    .sampling_mode(SamplingMode::Nearest),
            )
            .into_element(),
        None => rect()
            .width(Size::px(size))
            .height(Size::px(size))
            .corner_radius(8.)
            .center()
            .background(theme::rgba(ACCENT, 50))
            .color(theme::rgb(ACCENT))
            .child("DS")
            .into_element(),
    }
}

#[derive(PartialEq)]
pub struct Welcome;

impl Component for Welcome {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let config = ctx.config.read().clone();
        let mut roms: Vec<PathBuf> = config.recent_roms.clone();
        for rom in rom_paths(&config.rom_dir) {
            if !roms.contains(&rom) {
                roms.push(rom);
            }
        }
        roms.truncate(MAX_CARDS);
        let open_ctx = ctx.clone();

        ScrollView::new().child(
            rect()
                .width(Size::fill())
                .padding((36., 32.))
                .cross_align(Alignment::Center)
                .spacing(22.)
                .child(
                    rect()
                        .cross_align(Alignment::Center)
                        .spacing(10.)
                        .child(logo(64.))
                        .child(
                            rect()
                                .font_size(26.)
                                .font_weight(FontWeight::BOLD)
                                .color(theme::rgb(TEXT))
                                .child("Lunaris DS"),
                        )
                        .child(
                            rect()
                                .font_size(13.)
                                .color(theme::rgb(TEXT_DIM))
                                .child("Pick a game below, or open one from a file."),
                        ),
                )
                .child(
                    rect()
                        .horizontal()
                        .spacing(10.)
                        .child(
                            Button::new()
                                .filled()
                                .on_press(move |_| actions::run(&open_ctx, Action::OpenRom))
                                .child("Open ROM…   Ctrl+O"),
                        )
                        .child({
                            let ctx = ctx.clone();
                            Button::new()
                                .on_press(move |_| {
                                    actions::run(
                                        &ctx,
                                        Action::Open(ds_common::menu::Window::Paths),
                                    );
                                })
                                .child("ROM folder…")
                        }),
                )
                .maybe_child((!roms.is_empty()).then(|| {
                    rect()
                        .width(Size::px(760.))
                        .spacing(10.)
                        .child(rect().font_size(12.).color(theme::rgb(TEXT_FAINT)).child(
                            if config.recent_roms.is_empty() {
                                "ROM FOLDER"
                            } else {
                                "RECENT AND ROM FOLDER"
                            },
                        ))
                        .child(
                            rect()
                                .horizontal()
                                .width(Size::fill())
                                .content(Content::wrap())
                                .spacing(10.)
                                .children(roms.into_iter().enumerate().map(|(i, path)| {
                                    RomCard { path, ctx: ctx.clone() }.into_element_keyed(i)
                                })),
                        )
                })),
        )
    }
}

trait Keyed {
    fn into_element_keyed(self, key: usize) -> Element;
}

impl<C: Component + 'static> Keyed for C {
    fn into_element_keyed(self, key: usize) -> Element {
        rect().key(key).child(self).into_element()
    }
}

/// One game on the welcome screen.
struct RomCard {
    path: PathBuf,
    ctx: Ctx,
}

impl PartialEq for RomCard {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Component for RomCard {
    fn render(&self) -> impl IntoElement {
        let mut hover = use_state(|| false);
        let card = card(&self.path);
        let path = self.path.clone();
        let ctx = self.ctx.clone();
        rect()
            .width(Size::px(246.))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(12.)
            .padding(10.)
            .corner_radius(12.)
            .background(if hover() { theme::surface_hover() } else { theme::surface() })
            .border(Border::new().width(1.).fill(if hover() {
                theme::rgba(ACCENT, 140)
            } else {
                theme::hairline()
            }))
            .on_pointer_enter(move |_| hover.set(true))
            .on_pointer_leave(move |_| hover.set(false))
            .on_press(move |_| actions::run(&ctx, Action::OpenRomPath(path.clone())))
            .child(rom_icon(card.icon, 44.))
            .child(
                rect()
                    .width(Size::px(170.))
                    .spacing(3.)
                    .child(
                        label()
                            .text(card.title)
                            .max_lines(1)
                            .text_overflow(TextOverflow::Ellipsis)
                            .font_size(13.)
                            .color(theme::rgb(TEXT)),
                    )
                    .child(
                        label()
                            .text(file_name(&self.path))
                            .max_lines(1)
                            .text_overflow(TextOverflow::Ellipsis)
                            .font_size(11.)
                            .color(theme::rgb(TEXT_FAINT)),
                    ),
            )
    }
}

/// A small rounded label.
fn chip(text: String, color: (u8, u8, u8)) -> Element {
    rect()
        .padding((2., 9.))
        .corner_radius(9.)
        .background(theme::rgba(color, 38))
        .color(theme::rgb(color))
        .font_size(11.)
        .child(text)
        .into_element()
}

#[derive(PartialEq)]
pub struct StatusBar;

impl Component for StatusBar {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let status = ctx.status.read().clone();
        let view = ctx.config.read().view.clone();
        let error = ctx.error.read().clone();
        let mut error_state = ctx.error;

        let (state, color) = match (status.running, status.paused) {
            (true, true) => ("Paused", WARNING),
            (true, false) => ("Running", SUCCESS),
            (false, _) => ("Idle", TEXT_FAINT),
        };
        // Lunaris and melonDS render at 1x whatever the setting.
        let scale = if BACKEND.upscales() { view.render_scale() } else { 1 };
        let mut res = format!("{}×{}", 256 * u32::from(scale), 192 * u32::from(scale));
        if view.xbrz && scale >= 2 {
            res.push_str(" · xBRZ");
        }

        rect()
            .horizontal()
            .content(Content::Flex)
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(STATUSBAR_HEIGHT))
            .padding((0., 12.))
            .spacing(8.)
            .font_size(11.5)
            .color(theme::rgb(TEXT_DIM))
            .child(chip(format!("● {state}"), color))
            .child(match error {
                Some(e) => rect()
                    .width(Size::flex(1.))
                    .horizontal()
                    .spacing(8.)
                    .cross_align(Alignment::Center)
                    .color(theme::rgb(DANGER))
                    .child(label().text(e).max_lines(1).text_overflow(TextOverflow::Ellipsis))
                    .child(
                        rect()
                            .color(theme::rgb(TEXT_DIM))
                            .on_press(move |_| error_state.set(None))
                            .child("✕"),
                    )
                    .into_element(),
                None => rect()
                    .width(Size::flex(1.))
                    .child(
                        label()
                            .text(status.rom.clone().unwrap_or_else(|| "No game".to_owned()))
                            .max_lines(1)
                            .text_overflow(TextOverflow::Ellipsis),
                    )
                    .into_element(),
            })
            .maybe_child(status.running.then(|| chip(format!("{:.1} fps", status.fps), SUCCESS)))
            .child(chip(ds_common::speed::label(view.speed), ACCENT))
            .child(chip(res, theme::ACCENT_2))
            .child(chip(format!("⚙ {}", BACKEND), theme::ACCENT_2))
            .child(chip(format!("instance{}", status.instance), TEXT_FAINT))
    }
}
