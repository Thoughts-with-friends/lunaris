//! The windows the menus open (melonDS's dialogs), drawn as modal cards
//! over the screens.
//!
//! ```text
//!  Action::Open(Window) -> Ctx::dialog = Some(window)
//!  Dialogs: Modal { title, width } + the window's view (views.rs, cheats.rs)
//!  Escape / ✕ / a click outside -> Ctx::dialog = None
//! ```
mod cheats;
mod views;

use ds_common::menu::Window;
use freya::{animation::*, prelude::*};

use crate::{
    state::Ctx,
    theme::{self, TEXT, TEXT_DIM, TEXT_FAINT},
};

#[derive(PartialEq)]
pub struct Dialogs;

impl Component for Dialogs {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let open = *ctx.dialog.read();
        rect().maybe_child(open.map(|window| {
            let (title, width) = match window {
                Window::RomInfo => ("ROM info", 560.),
                Window::Multiplayer => ("Multiplayer", 600.),
                Window::EmuSettings => ("Emu settings", 600.),
                Window::Input => ("Input and hotkeys", 560.),
                Window::Audio => ("Audio settings", 420.),
                Window::Wifi => ("Wifi settings", 480.),
                Window::Paths => ("Path settings", 600.),
                Window::About => ("About", 440.),
                Window::Cheats => ("Cheats", 1080.),
            };
            let body: Element = match window {
                Window::RomInfo => views::RomInfoView.into_element(),
                Window::Multiplayer => crate::lan::LanView.into_element(),
                Window::EmuSettings => views::EmuSettingsView.into_element(),
                Window::Input => views::InputView.into_element(),
                Window::Audio => views::AudioView.into_element(),
                Window::Wifi => views::WifiView.into_element(),
                Window::Paths => views::PathsView.into_element(),
                Window::About => views::AboutView.into_element(),
                Window::Cheats => cheats::CheatsView.into_element(),
            };
            Modal { title: title.to_owned(), width, body }.into_element()
        }))
    }
}

/// A dimmed backdrop and a centred glass card with a title bar.
#[derive(PartialEq)]
struct Modal {
    title: String,
    width: f32,
    body: Element,
}

impl Component for Modal {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut dialog = ctx.dialog;
        let animation = use_animation(|conf| {
            conf.on_creation(OnCreation::Run);
            (
                AnimNum::new(0.94, 1.).time(220).ease(Ease::Out).function(Function::Expo),
                AnimNum::new(0., 1.).time(160).ease(Ease::Out).function(Function::Quad),
            )
        });
        let (scale, opacity) = &*animation.read();
        let (scale, opacity) = (scale.value(), opacity.value());
        // The body scrolls once it is taller than the window allows.
        let mut content_height = use_state(|| 0.0_f32);
        let window_height = Platform::get().current_window().root_size.read().height;
        let max_body = window_height.mul_add(0.9, -64.).max(120.);
        let body_height = content_height().clamp(1., max_body);

        let card = rect()
            .width(Size::px(self.width))
            .corner_radius(16.)
            .background(theme::rgba(theme::BODY, 248))
            .border(Border::new().width(1.).fill(theme::hairline()))
            .shadow(Shadow::new().y(18.).blur(48.).color((0, 0, 0, 150)))
            .color(theme::rgb(TEXT))
            .scale((scale, scale))
            .opacity(opacity)
            .on_press(|e: Event<PressEventData>| e.stop_propagation())
            .on_global_key_down(move |e: Event<KeyboardEventData>| {
                if e.key == Key::Named(NamedKey::Escape) {
                    dialog.set(None);
                }
            })
            .child(
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .width(Size::fill())
                    .padding((14., 14., 12., 20.))
                    .child(
                        rect()
                            .width(Size::flex(1.))
                            .font_size(16.)
                            .font_weight(FontWeight::BOLD)
                            .child(self.title.clone()),
                    )
                    .child(CloseButton),
            )
            .child(rect().width(Size::fill()).height(Size::px(1.)).background(theme::hairline()))
            .child(
                ScrollView::new().width(Size::fill()).height(Size::px(body_height)).child(
                    rect()
                        .width(Size::fill())
                        .padding((16., 20., 20., 20.))
                        .on_sized(move |e: Event<SizedEventData>| {
                            content_height.set_if_modified(e.area.height());
                        })
                        .child(self.body.clone()),
                ),
            );

        rect()
            .layer(Layer::Overlay)
            .position(Position::new_global())
            .child(
                rect()
                    .position(Position::new_global().top(0.).left(0.))
                    .width(Size::window_percent(100.))
                    .height(Size::window_percent(100.))
                    .background(Color::from_argb((110. * opacity) as u8, 4, 5, 12))
                    .on_press(move |_| dialog.set(None)),
            )
            .child(
                rect()
                    .position(Position::new_global().top(0.).left(0.))
                    .width(Size::window_percent(100.))
                    .height(Size::window_percent(100.))
                    .center()
                    .child(card),
            )
    }
}

#[derive(PartialEq)]
struct CloseButton;

impl Component for CloseButton {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut dialog = ctx.dialog;
        let mut hover = use_state(|| false);
        rect()
            .width(Size::px(28.))
            .height(Size::px(28.))
            .corner_radius(14.)
            .center()
            .background(if hover() {
                Color::from_argb(36, 255, 255, 255)
            } else {
                Color::TRANSPARENT
            })
            .color(theme::rgb(TEXT_DIM))
            .on_pointer_enter(move |_| hover.set(true))
            .on_pointer_leave(move |_| hover.set(false))
            .on_press(move |_| dialog.set(None))
            .child("✕")
    }
}

/// A small uppercase heading inside a dialog.
pub fn section(title: &str) -> Element {
    rect()
        .margin((10., 0., 6., 0.))
        .font_size(11.)
        .font_weight(FontWeight::BOLD)
        .color(theme::rgb(TEXT_FAINT))
        .child(title.to_uppercase())
        .into_element()
}

/// `label` on the left (fixed width), `value` on the right.
pub fn field(label: &str, value: impl IntoElement) -> Element {
    rect()
        .horizontal()
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .width(Size::fill())
        .padding((5., 0.))
        .child(
            rect()
                .width(Size::px(130.))
                .font_size(13.)
                .color(theme::rgb(TEXT_DIM))
                .child(label.to_owned()),
        )
        .child(rect().width(Size::flex(1.)).font_size(13.).child(value))
        .into_element()
}

/// A muted footnote.
pub fn note(text: &str) -> Element {
    rect()
        .margin((10., 0., 0., 0.))
        .font_size(11.5)
        .color(theme::rgb(TEXT_FAINT))
        .child(text.to_owned())
        .into_element()
}

/// A dim hint beside a control.
#[cfg(feature = "melon")]
pub fn hint(text: &str) -> Element {
    rect().font_size(11.5).color(theme::rgb(TEXT_DIM)).child(text.to_owned()).into_element()
}

/// `−  value  +`, then a hint.
#[cfg(feature = "melon")]
pub fn stepper(
    value: String,
    down: impl FnMut(Event<PressEventData>) + 'static,
    up: impl FnMut(Event<PressEventData>) + 'static,
    text: &str,
) -> Element {
    rect()
        .horizontal()
        .spacing(8.)
        .cross_align(Alignment::Center)
        .child(Button::new().compact().on_press(down).child("−"))
        .child(rect().width(Size::px(70.)).main_align(Alignment::Center).child(value))
        .child(Button::new().compact().on_press(up).child("+"))
        .child(hint(text))
        .into_element()
}

/// A `▸ title` line that opens and closes what follows it.
#[cfg(feature = "melon")]
pub fn fold(mut open: State<bool>, title: &str) -> Element {
    rect()
        .horizontal()
        .spacing(6.)
        .margin((10., 0., 0., 0.))
        .font_size(13.)
        .color(theme::rgb(TEXT_DIM))
        .on_press(move |_| {
            let now = *open.peek();
            open.set(!now);
        })
        .child(if *open.read() { "▾" } else { "▸" })
        .child(title.to_owned())
        .into_element()
}
