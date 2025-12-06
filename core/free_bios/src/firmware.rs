/*
  SPDX-FileCopyrightText: (C) 2007 kelpsyberry
  SPDX-License-Identifier: GPL-3.0-or-later
  https://github.com/kelpsyberry/dust/blob/main/core/src/spi/firmware.rs#L8
*/

/// DSType enum
/// DS, Lite, DSi, iQue DS, iQue DS Lite
#[derive(Clone, Copy)]
pub enum DSType {
    Ds,
    Lite,
    Dsi,
    Ique,
    IqueLite,
}

// CRC16 calculation using while loop, const compatible
pub const fn crc16(user: [u8; 0x70]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    let mut i = 0;
    while i < 0x70 {
        crc ^= user[i] as u16;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        i += 1;
    }
    crc
}

// Firmware length based on model
pub const fn firmware_len(model: DSType) -> usize {
    match model {
        DSType::Dsi => 0x2_0000,
        DSType::Ds | DSType::Lite => 0x4_0000,
        DSType::Ique | DSType::IqueLite => 0x8_0000,
    }
}

/// Const-compatible firmware creation
///
/// # Example
/// ```rust
/// use free_bios::firmware::{default_firmware, DSType};
/// const FIRMWARE: [u8; 0x40000] = default_firmware::<0x40000>(DSType::Ds);
/// ```
pub const fn default_firmware<const N: usize>(model: DSType) -> [u8; N] {
    let mut firmware = [0u8; N];

    // 0x04..0x07
    firmware[0x04] = 0x00;
    firmware[0x05] = 0xDB;
    firmware[0x06] = 0x1F;
    firmware[0x07] = 0x0F;

    // 0x08..0x0B "MACh"
    // TODO: optional customization
    firmware[0x08] = b'M';
    firmware[0x09] = b'A';
    firmware[0x0A] = b'C';
    firmware[0x0B] = 0x68;

    // 0x14..0x15
    let val14: u16 = ((N >> 17) << 12) as u16;
    firmware[0x14] = (val14 & 0xFF) as u8;
    firmware[0x15] = (val14 >> 8) as u8;

    // 0x18..0x1C
    firmware[0x18] = 0x00;
    firmware[0x19] = 0x00;
    firmware[0x1A] = 0x01;
    firmware[0x1B] = 0x01;
    firmware[0x1C] = 0x06;

    // 0x1D model-specific
    firmware[0x1D] = match model {
        DSType::Ds => 0xFF,
        DSType::Lite => 0x20,
        DSType::Ique => 0x57,
        DSType::IqueLite => 0x43,
        DSType::Dsi => 0x63,
    };

    // 0x1E..0x1F
    firmware[0x1E] = 0xFF;
    firmware[0x1F] = 0xFF;

    // 0x20..0x28 values
    let values: [u16; 5] = [((N - 0x200) >> 3) as u16, 0x0B51, 0x0DB3, 0x4F5D, 0xFFFF];
    let mut idx = 0;
    while idx < 5 {
        let val = values[idx];
        firmware[0x20 + idx * 2] = (val & 0xFF) as u8;
        firmware[0x21 + idx * 2] = (val >> 8) as u8;
        idx += 1;
    }

    // User settings for 2 users
    let mut u = 0;
    while u < 2 {
        let start = N - 0x200 + u * 0x100;
        let mut i = 0;
        while i < 0x100 {
            firmware[start + i] = 0;
            i += 1;
        }

        firmware[start + 0x00] = 5;
        firmware[start + 0x02] = if u == 0 { 1 } else { 0 };
        firmware[start + 0x03] = 1;
        firmware[start + 0x04] = 1;

        // Name "Luna"
        firmware[start + 0x06] = b'L';
        firmware[start + 0x07] = 0;
        firmware[start + 0x08] = b'u';
        firmware[start + 0x09] = 0;
        firmware[start + 0x0A] = b'n';
        firmware[start + 0x0B] = 0;
        firmware[start + 0x0C] = b'a';
        firmware[start + 0x0D] = 0;

        // CRC16
        let mut crc: [u8; 112] = [0; 0x70];
        let mut i = 0;
        while i < 0x70 {
            crc[i] = firmware[start + i];
            i += 1;
        }
        let crc: u16 = crc16(crc);

        firmware[start + 0x72] = (crc & 0xFF) as u8;
        firmware[start + 0x73] = (crc >> 8) as u8;

        u += 1;
    }

    firmware
}
