//! The two DS screens: frames from the emulator thread, placed by the
//! shared View settings, and the stylus.
//!
//! ```text
//!  emulator thread --Shared::frame_since--> frame pump (async, ~200 Hz poll)
//!     RGBA upper/lower (256x192 x internal resolution)
//!       -> ImageHandle x2 in this component's own state (only the screens
//!          re-render at 60 Hz, not the menus or dialogs)
//!  area size --ds_common::view::place--> ScreenRect per screen
//!     rect (rotated) + image()      pointer -> ScreenRect::to_ds -> set_touch
//! ```
use std::time::Duration;

use async_io::Timer;
use bytes::Bytes;
use ds_common::view::{Placement, ScreenRect, place};
use freya::{elements::image::ImageHandle, engine::prelude::AlphaType, prelude::*};
use futures_lite::StreamExt;

use crate::{
    state::Ctx,
    theme::{self, SUCCESS},
};

/// How often the frame pump looks for a new frame.
const FRAME_POLL: Duration = Duration::from_millis(4);

/// One decoded frame: both screens.
#[derive(Clone, PartialEq)]
struct Frame {
    upper: ImageHandle,
    lower: ImageHandle,
}

/// `w` x `h` RGBA pixels turned `degrees` clockwise (0, 90, 180, 270);
/// returns the new width and height. (freya's `rotate` does not turn an
/// image's pixels, so the frames are turned here.)
pub fn rotate_rgba(src: &[u8], w: usize, h: usize, degrees: u16) -> (Vec<u8>, usize, usize) {
    if degrees == 0 {
        return (src.to_vec(), w, h);
    }
    let px = src.as_chunks::<4>().0;
    let (ow, oh) = if degrees == 180 { (w, h) } else { (h, w) };
    let mut out = vec![0_u8; ow * oh * 4];
    for (i, dst) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % ow, i / ow);
        let (sx, sy) = match degrees {
            90 => (y, h - 1 - x),
            180 => (w - 1 - x, h - 1 - y),
            _ => (w - 1 - y, x),
        };
        *dst = px[sy * w + sx];
    }
    (out, ow, oh)
}

/// Splits `get_frame`-style RGBA (upper then lower, `width` wide) into two
/// images, turned `rotation` degrees clockwise.
fn decode(width: u32, rgba: &[u8], rotation: u16) -> Option<Frame> {
    let half = rgba.len() / 2;
    let w = width.max(1) as usize;
    let h = half / 4 / w;
    let image = |part: &[u8]| {
        let (pixels, ow, oh) = rotate_rgba(part, w, h, rotation);
        ImageHandle::from_rgba(
            u32::try_from(ow).ok()?,
            u32::try_from(oh).ok()?,
            Bytes::from(pixels),
            AlphaType::Opaque,
        )
    };
    Some(Frame { upper: image(&rgba[..half])?, lower: image(&rgba[half..])? })
}

#[derive(PartialEq)]
pub struct Screens;

impl Component for Screens {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let mut frame = use_state(|| None::<Frame>);
        let mut area = use_state(Area::default);
        let mut touching = use_state(|| false);

        // Frame pump.
        let pump_ctx = ctx.clone();
        use_hook(move || {
            spawn(async move {
                let mut last = 0;
                let mut tick = Timer::interval(FRAME_POLL);
                loop {
                    tick.next().await;
                    if let Some((seq, width, rgba)) = pump_ctx.front.shared.frame_since(last) {
                        last = seq;
                        let rotation = pump_ctx.config.peek().view.rotation();
                        if let Some(f) = decode(width, &rgba, rotation) {
                            frame.set(Some(f));
                        }
                    }
                }
            });
        });

        let config = ctx.config.read().clone();
        let running = ctx.status.read().running;
        let show_fps = config.view.show_fps;
        let a = area();
        let placement = place(&config.view, a.width(), a.height());
        let bottom = placement.screens[1];

        // Stylus: pointer position inside the area -> DS pixel.
        let touch_ctx = ctx;
        let touch_at = move |x: f32, y: f32, clamp: bool| -> Option<(i32, i32)> {
            let screen = bottom?;
            let r = screen.rect;
            let (x, y) = if clamp {
                (x.clamp(r.x, r.x + r.w - 0.01), y.clamp(r.y, r.y + r.h - 0.01))
            } else {
                (x, y)
            };
            screen.to_ds(x, y)
        };
        let press_ctx = touch_ctx.clone();
        let on_pointer_down = move |e: Event<PointerEventData>| {
            // Relative to the area, whichever child was hit.
            let g = e.global_location();
            let origin = area.peek().origin;
            if let Some(pos) = touch_at(g.x as f32 - origin.x, g.y as f32 - origin.y, false) {
                touching.set(true);
                press_ctx.front.shared.set_touch(Some(pos));
            }
        };
        let move_ctx = touch_ctx.clone();
        let on_pointer_move = move |e: Event<PointerEventData>| {
            if !touching() {
                return;
            }
            let g = e.global_location();
            let origin = area.peek().origin;
            let (x, y) = (g.x as f32 - origin.x, g.y as f32 - origin.y);
            if let Some(pos) = touch_at(x, y, true) {
                move_ctx.front.shared.set_touch(Some(pos));
            }
        };
        let up_ctx = touch_ctx;
        let on_pointer_up = move |_: Event<PointerEventData>| {
            if touching() {
                touching.set(false);
                up_ctx.front.shared.set_touch(None);
            }
        };

        let sampling =
            if config.view.filtering { SamplingMode::Bilinear } else { SamplingMode::Nearest };
        let scale = f32::from(config.view.render_scale());

        rect()
            .expanded()
            .on_sized(move |e: Event<SizedEventData>| area.set_if_modified(e.area))
            .on_pointer_down(on_pointer_down)
            .on_global_pointer_move(on_pointer_move)
            .on_global_pointer_up(on_pointer_up)
            .maybe_child(frame.read().clone().filter(|_| running).map(|f| {
                rect().expanded().children(
                    [(placement.screens[0], f.upper), (placement.screens[1], f.lower)]
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, (screen, img))| {
                            screen.map(|s| screen_view(i, s, img, sampling.clone(), scale))
                        }),
                )
            }))
            .maybe_child((running && show_fps).then_some(Osd))
            .maybe_child(placement_hint(&placement, running))
    }
}

/// Nothing to draw while the area has not been measured yet.
fn placement_hint(placement: &Placement, running: bool) -> Option<Element> {
    (running && placement.screens.iter().all(Option::is_none)).then(|| {
        rect()
            .expanded()
            .center()
            .color(theme::rgb(theme::TEXT_FAINT))
            .child("Both screens are hidden (View > Screen sizing)")
            .into_element()
    })
}

/// One screen (its image already turned) in its box, with a soft shadow.
fn screen_view(
    key: usize,
    s: ScreenRect,
    img: ImageHandle,
    sampling: SamplingMode,
    scale: f32,
) -> Element {
    let r = s.rect;
    // A bitmap larger than its box is always shrunk smoothly.
    let native = if s.rotation.is_multiple_of(180) { 256. } else { 192. };
    let sampling = if r.w < native * scale { SamplingMode::Trilinear } else { sampling };
    rect()
        .key(key)
        .position(Position::new_absolute().left(r.x).top(r.y))
        .width(Size::px(r.w))
        .height(Size::px(r.h))
        .corner_radius(4.)
        .overflow(Overflow::Clip)
        .background(Color::BLACK)
        .shadow(Shadow::new().y(6.).blur(22.).color((0, 0, 0, 110)))
        .child(image(img).expanded().aspect_ratio(AspectRatio::None).sampling_mode(sampling))
        .into_element()
}

/// Frame rate (and speed) in the top-left corner.
#[derive(PartialEq)]
struct Osd;

impl Component for Osd {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let status = ctx.status.read().clone();
        let speed = ctx.config.read().view.speed;
        let text = if status.paused {
            format!("{:.1} fps  ·  paused", status.fps)
        } else if speed == ds_common::speed::NORMAL {
            format!("{:.1} fps", status.fps)
        } else {
            format!("{:.1} fps  ·  {}", status.fps, ds_common::speed::label(speed))
        };
        rect()
            .position(Position::new_absolute().left(8.).top(8.))
            .padding((3., 9.))
            .corner_radius(10.)
            .background(Color::from_argb(150, 0, 0, 0))
            .color(theme::rgb(SUCCESS))
            .font_size(11.5)
            .font_family("Consolas")
            .child(text)
    }
}

#[cfg(test)]
mod tests {
    use super::rotate_rgba;

    #[test]
    fn rotation_turns_pixels_clockwise() {
        // A row "1 2" turned clockwise becomes a column with 1 on top (the
        // same turn ScreenRect::to_ds undoes for the stylus).
        let src = [1, 0, 0, 0, 2, 0, 0, 0];
        let (out, w, h) = rotate_rgba(&src, 2, 1, 90);
        assert_eq!((w, h), (1, 2));
        assert_eq!([out[0], out[4]], [1, 2]);
        let (out, ..) = rotate_rgba(&src, 2, 1, 270);
        assert_eq!([out[0], out[4]], [2, 1]);
        let (out, ..) = rotate_rgba(&src, 2, 1, 180);
        assert_eq!([out[0], out[4]], [2, 1]);
    }
}
