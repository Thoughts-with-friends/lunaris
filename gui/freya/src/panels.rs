//! The welcome screen (no game running) and the status bar.
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use ds_common::{
    frontend::{file_name, rom_paths},
    menu::{Action, Window},
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
            (true, true) => ("Paused · Resume", WARNING),
            (true, false) => ("Running · Pause", SUCCESS),
            (false, _) => ("Idle", TEXT_FAINT),
        };
        // Remote Desktop changes on its own threads: looked at twice a second.
        let mut tick = use_state(|| 0_u32);
        use_hook(move || {
            spawn(async move {
                loop {
                    async_io::Timer::after(std::time::Duration::from_millis(500)).await;
                    *tick.write() += 1;
                }
            });
        });
        let _ = tick.read();
        let limited = ctx.config.read().limit_framerate;
        // Every item is a button changing what it shows (or opening where it
        // is set).
        let remote = ctx.front.remote_state().map(|r| {
            let (text, color) = match r {
                crate::remote::RemoteState::Waiting { hosting: true } => {
                    ("⇄ Remote: waiting for a client".to_owned(), WARNING)
                }
                crate::remote::RemoteState::Waiting { hosting: false } => {
                    ("⇄ Remote: connecting…".to_owned(), WARNING)
                }
                crate::remote::RemoteState::Connected { hosting, peer } => (
                    format!("⇄ Remote: {} {peer}", if hosting { "client" } else { "host" }),
                    SUCCESS,
                ),
            };
            button(&ctx, chip(text, color), Action::Open(Window::Multiplayer))
        });
        let state_button = button(
            &ctx,
            chip(format!("● {state}"), color),
            if status.running { Action::TogglePause } else { Action::OpenRom },
        );
        let game = button(
            &ctx,
            label()
                .text(status.rom.clone().unwrap_or_else(|| "No game · Open ROM…".to_owned()))
                .max_lines(1)
                .text_overflow(TextOverflow::Ellipsis)
                .into_element(),
            if status.running { Action::Open(Window::RomInfo) } else { Action::OpenRom },
        );
        let fps = status.running.then(|| {
            let text = if limited {
                format!("{:.1} fps", status.fps)
            } else {
                format!("{:.1} fps · unlimited", status.fps)
            };
            button(&ctx, chip(text, SUCCESS), Action::ToggleLimitFramerate)
        });
        let volume = ctx.config.read().volume;
        let mut popup = ctx.volume_popup;
        let volume = rect()
            .child(
                rect()
                    .on_press(move |e: Event<PressEventData>| {
                        e.stop_propagation();
                        popup.toggle();
                    })
                    .child(chip(
                        if volume == 0 { "♪ muted".to_owned() } else { format!("♪ {volume}%") },
                        if volume == 0 { TEXT_FAINT } else { ACCENT },
                    )),
            )
            .maybe_child(popup().then(|| {
                rect().width(Size::px(0.)).height(Size::px(0.)).child(
                    rect()
                        .position(Position::new_absolute().top(6.).right(0.))
                        .layer(Layer::Overlay)
                        .child(VolumePanel),
                )
            }))
            .into_element();
        let speed =
            button(&ctx, chip(ds_common::speed::label(view.speed), ACCENT), Action::CycleSpeed);
        // Lunaris and melonDS render above 1x; the chip steps 1x..4x.
        let scale = if BACKEND.upscales() { view.render_scale() } else { 1 };
        let res = format!("{}×{}", 256 * u32::from(scale), 192 * u32::from(scale));
        let next_scale = scale % ds_common::view::MAX_RENDER_SCALE + 1;
        let res = button(&ctx, chip(res, theme::ACCENT_2), Action::SetRenderScale(next_scale));
        let xbrz = BACKEND.upscales().then(|| {
            let (text, color) = if view.xbrz && scale >= 2 {
                ("xBRZ on", theme::ACCENT_2)
            } else {
                ("xBRZ off", TEXT_FAINT)
            };
            button(&ctx, chip(text.to_owned(), color), Action::ToggleXbrz)
        });
        let backend = button(
            &ctx,
            chip(format!("⚙ {BACKEND}"), theme::ACCENT_2),
            Action::Open(Window::EmuSettings),
        );
        let instance = button(
            &ctx,
            chip(format!("instance{}", status.instance), TEXT_FAINT),
            Action::OpenDirectory,
        );

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
            .child(state_button)
            .maybe_child(remote)
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
                None => rect().width(Size::flex(1.)).child(game).into_element(),
            })
            .maybe_child(fps)
            .child(volume)
            .child(speed)
            .child(res)
            .maybe_child(xbrz)
            .child(backend)
            .child(instance)
    }
}

/// The status bar's volume popover (as the titlebar's Appearance).
#[derive(PartialEq)]
struct VolumePanel;

impl Component for VolumePanel {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let volume = ctx.config.read().volume;
        let (slider_ctx, mute_ctx) = (ctx.clone(), ctx.clone());
        let mut popup = ctx.volume_popup;
        rect()
            .width(Size::px(240.))
            .padding(14.)
            .spacing(10.)
            .corner_radius(14.)
            .background(theme::rgba(theme::BODY, 245))
            .border(Border::new().width(1.).fill(theme::hairline()))
            .shadow(Shadow::new().y(8.).blur(24.).color((0, 0, 0, 120)))
            .color(theme::rgb(theme::TEXT))
            .font_size(12.5)
            .on_press(|e: Event<PressEventData>| e.stop_propagation())
            .child(
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .width(Size::fill())
                    .child(
                        rect().width(Size::flex(1.)).font_weight(FontWeight::BOLD).child("Volume"),
                    )
                    .child(rect().color(theme::rgb(TEXT_DIM)).child(format!("{volume}%")))
                    .child(
                        rect()
                            .margin((0., 0., 0., 10.))
                            .color(theme::rgb(TEXT_FAINT))
                            .on_press(move |_| popup.set(false))
                            .child("✕"),
                    ),
            )
            .child(
                Slider::new(move |v: f64| {
                    slider_ctx.update_config(|c| c.volume = v.round().clamp(0., 100.) as u8);
                })
                .value(f64::from(volume))
                .size(Size::fill()),
            )
            .child(
                Button::new()
                    .on_press(move |_| {
                        mute_ctx.update_config(|c| c.volume = if c.volume == 0 { 80 } else { 0 });
                    })
                    .child(if volume == 0 { "Unmute" } else { "Mute" }),
            )
            .into_element()
    }
}

/// `content` as a button running `action` (as the menu entry would).
fn button(ctx: &Ctx, content: Element, action: Action) -> Element {
    let ctx = ctx.clone();
    rect()
        .on_press(move |_| crate::actions::run(&ctx, action.clone()))
        .child(content)
        .into_element()
}
