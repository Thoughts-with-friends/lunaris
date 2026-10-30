// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
pub const THUMB_INST_WIDTH: u32 = 2;
pub const ARM_INST_WIDTH: u32 = 4;

pub mod arm_cpu;
pub mod coprocessor_15;
pub mod disassemble;
pub mod instruction_table;
