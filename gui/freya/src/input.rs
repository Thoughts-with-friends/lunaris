//! Keyboard, gamepads and the periodic status refresh.
//!
//! ```text
//!  key down / up (root, global) --code--> keyboard bits ─┐
//!  gilrs (polled every 8 ms) ----------> gamepad bits ───┴-> Shared::set_keys
//!  left stick press ------------------> Action::CycleSpeed
//!  Ctrl+letter -----------------------> Action::from_shortcut
//! ```
//! DS keys are ignored while a dialog is open, so typing a cheat name does
//! not press buttons.
use std::time::Duration;

use async_io::Timer;
use ds_common::{Gamepads, emu_thread::key, menu::Action};
use freya::prelude::*;
use futures_lite::StreamExt;

use crate::{actions, state::Ctx};

const POLL: Duration = Duration::from_millis(8);
/// Polls between status refreshes (~0.5 s).
const STATUS_EVERY: u32 = 60;

/// The DS button of a keyboard key (same layout as the other GUIs).
pub const fn key_bit(code: &Code) -> Option<u32> {
    Some(match code {
        Code::ArrowUp => key::UP,
        Code::ArrowDown => key::DOWN,
        Code::ArrowLeft => key::LEFT,
        Code::ArrowRight => key::RIGHT,
        Code::KeyX => key::A,
        Code::KeyZ => key::B,
        Code::KeyS => key::X,
        Code::KeyA => key::Y,
        Code::KeyQ => key::L,
        Code::KeyW => key::R,
        Code::Enter => key::START,
        Code::Backspace => key::SELECT,
        _ => return None,
    })
}

/// The letter of a Ctrl shortcut key (by physical key, so the layout and
/// what Ctrl does to the typed character do not matter).
const fn shortcut_letter(code: &Code) -> Option<char> {
    Some(match code {
        Code::KeyO => 'O',
        Code::KeyP => 'P',
        Code::KeyR => 'R',
        Code::KeyN => 'N',
        Code::KeyQ => 'Q',
        _ => return None,
    })
}

/// Root key-down handler: shortcuts, then DS buttons.
pub fn on_key_down(ctx: &Ctx, e: &Event<KeyboardEventData>) {
    if ctx.dialog.peek().is_some() {
        return;
    }
    if e.modifiers.contains(Modifiers::CONTROL) {
        if let Some(action) = shortcut_letter(&e.code).and_then(Action::from_shortcut) {
            actions::run(ctx, action);
        }
        return;
    }
    if let Some(bit) = key_bit(&e.code) {
        ctx.keyboard.set(ctx.keyboard.get() | bit);
    }
}

pub fn on_key_up(ctx: &Ctx, e: &Event<KeyboardEventData>) {
    if let Some(bit) = key_bit(&e.code) {
        ctx.keyboard.set(ctx.keyboard.get() & !bit);
    }
}

/// Starts the input / status loop (once, from the root component).
pub fn spawn_loop(ctx: Ctx) {
    spawn(async move {
        let mut pads = Gamepads::new();
        let mut tick = Timer::interval(POLL);
        let mut last_keys = u32::MAX;
        let mut n = 0_u32;
        loop {
            tick.next().await;
            pads.poll();
            let dialog_open = ctx.dialog.peek().is_some();
            if dialog_open {
                ctx.keyboard.set(0);
            }
            let keys = ctx.keyboard.get() | pads.key_bits();
            if keys != last_keys {
                last_keys = keys;
                ctx.front.shared.set_keys(keys);
            }
            for _ in 0..pads.take_speed_presses() {
                if ctx.status.peek().running {
                    actions::run(&ctx, Action::CycleSpeed);
                }
            }
            n += 1;
            if n.is_multiple_of(STATUS_EVERY) {
                ctx.refresh_status();
                let names = pads.names();
                let mut gamepads = ctx.gamepads;
                if *gamepads.peek() != names {
                    gamepads.set(names);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_layout_matches_the_other_guis() {
        assert_eq!(key_bit(&Code::KeyX), Some(key::A));
        assert_eq!(key_bit(&Code::KeyA), Some(key::Y));
        assert_eq!(key_bit(&Code::Enter), Some(key::START));
        assert_eq!(key_bit(&Code::KeyP), None);
        assert_eq!(
            shortcut_letter(&Code::KeyO).and_then(Action::from_shortcut),
            Some(Action::OpenRom)
        );
    }
}
