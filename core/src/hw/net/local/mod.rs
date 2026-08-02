// SPDX-FileCopyrightText: (C) 2016-2026 melonDS team
// SPDX-License-Identifier: GPL-3.0
//! Local wireless play: MP frames exchanged between emulator instances
//! without leaving the machine.
//!
//! Corresponds to melonDS's `src/net/LocalMP.{h,cpp}`. The socket-backed
//! counterpart of this module (melonDS's `LAN.cpp`) deliberately lives in
//! the frontend crate `lunaris-net`, because `nds-core` owns no sockets —
//! see [`crate::hw::net`].

mod local_mp;
mod semaphore;

pub use local_mp::{
    LocalMp, LocalMpHub, MAX_FRAME_SIZE, MpStatusData, PACKET_QUEUE_SIZE, REPLY_QUEUE_SIZE,
};
pub use semaphore::Semaphore;
