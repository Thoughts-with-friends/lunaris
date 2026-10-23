//! The custom titlebar of the borderless window: logo, menu bar, the
//! running game (drag area), the Appearance popover and window buttons.
use freya::prelude::*;

use crate::{
    actions::{self, TITLEBAR_HEIGHT},
    menubar::MenuBar,
    state::Ctx,
    theme::{self, ACCENT, ACCENT_2, TEXT, TEXT_DIM, TEXT_FAINT},
};

/// A small gradient badge with the app's initial.
pub fn logo(size: f32) -> Element {
    rect()
        .width(Size::px(size))
        .height(Size::px(size))
        .corner_radius(size * 0.3)
        .center()
        .background(
            LinearGradient::new()
                .angle(135.)
                .stop((theme::rgb(ACCENT), 0.))
                .stop((theme::rgb(ACCENT_2), 100.)),
        )
        .shadow(Shadow::new().y(2.).blur(8.).color(theme::rgba(ACCENT, 90)))
        .color(Color::WHITE)
        .font_weight(FontWeight::BOLD)
        .font_size(size * 0.55)
        .child("L")
        .into_element()
}

#[derive(PartialEq)]
pub struct Titlebar;

impl Component for Titlebar {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let menus = ctx.menus.read().clone();
        let status = ctx.status.read().clone();
        let mut appearance = ctx.appearance;
        let mut maximized = use_state(|| false);

        use_side_effect(move || {
            let _ = Platform::get().current_window().root_size.read();
            Platform::get().with_window(Platform::window_id(), move |window| {
                maximized.set_if_modified(window.is_maximized());
            });
        });

        let title = match (&status.rom, status.paused) {
            (Some(rom), true) => format!("{rom}  ·  paused"),
            (Some(rom), false) => rom.clone(),
            (None, _) => format!("instance{}", status.instance),
        };

        let quit_ctx = ctx;
        rect()
            .horizontal()
            .content(Content::Flex)
            .width(Size::fill())
            .height(Size::px(TITLEBAR_HEIGHT))
            .cross_align(Alignment::Center)
            .padding((0., 6., 0., 12.))
            .spacing(10.)
            .child(logo(22.))
            .child(
                rect()
                    .font_weight(FontWeight::BOLD)
                    .font_size(13.5)
                    .color(theme::rgb(TEXT))
                    .child("Lunaris DS"),
            )
            .child(MenuBar { menus })
            .child(
                rect()
                    .window_drag()
                    .width(Size::flex(1.))
                    .height(Size::fill())
                    .center()
                    .color(theme::rgb(TEXT_FAINT))
                    .font_size(12.5)
                    .child(label().text(title).max_lines(1).text_overflow(TextOverflow::Ellipsis)),
            )
            .child(
                rect()
                    .child(IconButton {
                        glyph: "◐",
                        tip: "Appearance",
                        active: appearance(),
                        on_press: EventHandler::new(move |()| appearance.toggle()),
                    })
                    .maybe_child(appearance().then(|| {
                        rect().width(Size::px(0.)).height(Size::px(0.)).child(
                            rect()
                                .position(Position::new_absolute().top(6.).right(-36.))
                                .layer(Layer::Overlay)
                                .child(AppearancePanel),
                        )
                    })),
            )
            .child(TitlebarButton::new(TitlebarAction::Minimize).on_press(|_| {
                Platform::get().with_window(Platform::window_id(), |w| w.set_minimized(true));
            }))
            .child(
                TitlebarButton::new(if maximized() {
                    TitlebarAction::Restore
                } else {
                    TitlebarAction::Maximize
                })
                .on_press(|_| {
                    Platform::get().with_window(Platform::window_id(), |w| {
                        w.set_maximized(!w.is_maximized());
                    });
                }),
            )
            .child(
                TitlebarButton::new(TitlebarAction::Close)
                    .on_press(move |_| actions::quit(&quit_ctx)),
            )
    }
}

/// A round, borderless icon button for the titlebar.
#[derive(PartialEq)]
struct IconButton {
    glyph: &'static str,
    tip: &'static str,
    active: bool,
    on_press: EventHandler<()>,
}

impl Component for IconButton {
    fn render(&self) -> impl IntoElement {
        let mut hover = use_state(|| false);
        let on_press = self.on_press.clone();
        let background = if self.active {
            theme::rgba(ACCENT, 80)
        } else if hover() {
            Color::from_argb(30, 255, 255, 255)
        } else {
            Color::TRANSPARENT
        };
        TooltipContainer::new(Tooltip::new_text(self.tip)).position(AttachedPosition::Bottom).child(
            rect()
                .width(Size::px(30.))
                .height(Size::px(30.))
                .corner_radius(15.)
                .center()
                .background(background)
                .color(theme::rgb(TEXT_DIM))
                .font_size(15.)
                .on_pointer_enter(move |_| hover.set(true))
                .on_pointer_leave(move |_| hover.set(false))
                .on_press(move |e: Event<PressEventData>| {
                    e.stop_propagation();
                    on_press.call(());
                })
                .child(self.glyph),
        )
    }
}

/// Window opacity and the theme accents (freya.json).
#[derive(PartialEq)]
struct AppearancePanel;

impl Component for AppearancePanel {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let opacity = ctx.look.read().opacity;
        let slider_ctx = ctx.clone();
        let mut appearance = ctx.appearance;

        rect()
            .width(Size::px(260.))
            .padding(16.)
            .spacing(12.)
            .corner_radius(14.)
            .background(theme::rgba(theme::BODY, 245))
            .border(Border::new().width(1.).fill(theme::hairline()))
            .shadow(Shadow::new().y(8.).blur(24.).color((0, 0, 0, 120)))
            .color(theme::rgb(TEXT))
            .on_press(|e: Event<PressEventData>| e.stop_propagation())
            .child(
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .width(Size::fill())
                    .child(
                        rect()
                            .width(Size::flex(1.))
                            .font_weight(FontWeight::BOLD)
                            .child("Appearance"),
                    )
                    .child(
                        rect()
                            .color(theme::rgb(TEXT_FAINT))
                            .on_press(move |_| appearance.set(false))
                            .child("✕"),
                    ),
            )
            .child(
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .width(Size::fill())
                    .font_size(12.5)
                    .color(theme::rgb(TEXT_DIM))
                    .child(rect().width(Size::flex(1.)).child("Window opacity"))
                    .child(format!("{:.0}%", opacity * 100.)),
            )
            .child(
                Slider::new(move |v: f64| {
                    slider_ctx.update_look(|l| {
                        l.opacity = (v as f32 / 100.).mul_add(0.7, 0.3).clamp(0.3, 1.);
                    });
                })
                .value(f64::from((opacity - 0.3) / 0.7 * 100.))
                .size(Size::fill()),
            )
            .child(
                rect()
                    .font_size(11.5)
                    .color(theme::rgb(TEXT_FAINT))
                    .child("The desktop shows through the window; the DS screens stay opaque."),
            )
    }
}
