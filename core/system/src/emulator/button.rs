// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
//! Button state setters for the frontends.
//!
//! ```text
//! from: frontend input (gui/egui input.rs apply_keyboard, gamepads in
//!       gui/common, gui/tauri commands, headless PRESS=) before run()
//!   |
//!   v
//! button_*_pressed / button_*_released
//!   A B SELECT START arrows R L -> key_input  (KEYINPUT 0x04000130,
//!                                              read by both CPUs)
//!   X Y                         -> ext_key_in (EXTKEYIN 0x04000136,
//!                                              ARM7 only)
//!   |
//!   v
//! to:   the game's key polling (ARM7 copies X/Y/pen/hinge to the ARM9
//!       through shared RAM 0x027FFFA8)
//! ```
use crate::emulator::Emulator;

impl Emulator {
    /// Handle up button press
    pub fn button_up_pressed(&mut self) {
        self.key_input.up = true;
    }

    /// Handle down button press
    pub fn button_down_pressed(&mut self) {
        self.key_input.down = true;
    }

    /// Handle left button press
    pub fn button_left_pressed(&mut self) {
        self.key_input.left = true;
    }

    /// Handle start button press
    pub fn button_start_pressed(&mut self) {
        self.key_input.start = true;
    }

    /// Handle select button press
    pub fn button_select_pressed(&mut self) {
        self.key_input.select = true;
    }

    /// Handle A button press
    pub fn button_a_pressed(&mut self) {
        self.key_input.button_a = true;
    }

    /// Handle B button press
    pub fn button_b_pressed(&mut self) {
        self.key_input.button_b = true;
    }

    /// Handle X button press
    pub fn button_x_pressed(&mut self) {
        self.ext_key_in.button_x = true;
    }

    /// Handle Y button press
    pub fn button_y_pressed(&mut self) {
        self.ext_key_in.button_y = true;
    }

    /// Handle L button press
    pub fn button_l_pressed(&mut self) {
        self.key_input.button_l = true;
    }

    /// Handle R button press
    pub fn button_r_pressed(&mut self) {
        self.key_input.button_r = true;
    }

    /// Handle right button press
    pub fn button_right_pressed(&mut self) {
        self.key_input.right = true;
    }
}

impl Emulator {
    /// Handle up button release
    pub fn button_up_released(&mut self) {
        self.key_input.up = false;
    }

    /// Handle down button release
    pub fn button_down_released(&mut self) {
        self.key_input.down = false;
    }

    /// Handle left button release
    pub fn button_left_released(&mut self) {
        self.key_input.left = false;
    }

    /// Handle right button release
    pub fn button_right_released(&mut self) {
        self.key_input.right = false;
    }

    /// Handle start button release
    pub fn button_start_released(&mut self) {
        self.key_input.start = false;
    }

    /// Handle select button release
    pub fn button_select_released(&mut self) {
        self.key_input.select = false;
    }

    /// Handle A button release
    pub fn button_a_released(&mut self) {
        self.key_input.button_a = false;
    }

    /// Handle B button release
    pub fn button_b_released(&mut self) {
        self.key_input.button_b = false;
    }

    /// Handle X button release
    pub fn button_x_released(&mut self) {
        self.ext_key_in.button_x = false;
    }

    /// Handle Y button release
    pub fn button_y_released(&mut self) {
        self.ext_key_in.button_y = false;
    }

    /// Handle L button release
    pub fn button_l_released(&mut self) {
        self.key_input.button_l = false;
    }

    /// Handle R button release
    pub fn button_r_released(&mut self) {
        self.key_input.button_r = false;
    }
}
