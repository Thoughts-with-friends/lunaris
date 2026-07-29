// SPDX-FileCopyrightText: (C) 2007 kelpsyberry
// SPDX-License-Identifier: GPL-3.0-or-later
// https://github.com/kelpsyberry/dust/blob/main/core/src/spi/firmware.rs#L8

/// DSType enum
/// DS, Lite, DSi, iQue DS, iQue DS Lite
#[derive(Clone, Copy)]
pub enum DSType {
    Ds,
    Lite,
    Dsi,
    /// iQue: Chinese Nintendo DS, uses region-locked cartridges and localized OS
    Ique,
    IqueLite,
}

impl DSType {
    const fn model_spec(&self) -> u8 {
        match self {
            Self::Ds => 0xFF,
            Self::Lite => 0x20,
            Self::Ique => 0x57,
            Self::IqueLite => 0x43,
            Self::Dsi => 0x63,
        }
    }
}

///
/// # Example
/// ```rust
/// use free_bios::firmware::FIRMWARE_DS;
/// std::fs::write("firmware.bin", &FIRMWARE_DS).unwrap();
/// ```
/// DS firmware for standard DS/Lite (256KB)
pub static FIRMWARE_DS: [u8; 0x40000] = default_firmware::<0x40000>(DSType::Ds);
/// DS firmware for DSi (128KB)
pub static FIRMWARE_DSI: [u8; 0x20000] = default_firmware::<0x20000>(DSType::Dsi);
/// DS firmware for iQue DS (512KB)
pub static FIRMWARE_DS_IQUE: [u8; 0x80000] = default_firmware::<0x80000>(DSType::Ique);

// CRC16 calculation using while loop, const compatible. User-settings CRC
// starts from seed 0xFFFF (melonDS `UserData::UpdateChecksum` ->
// `CRC16(Bytes, 0x70, 0xFFFF)`).
const fn crc16(user: [u8; 0x70]) -> u16 {
    crc16_seeded(&user, 0xFFFF)
}

/// Same CRC16 (poly 0xA001, the standard "CRC-16/ARC" used throughout DS
/// firmware) over an arbitrary-length slice with an explicit start value,
/// const-compatible. melonDS's `Firmware::CRC16(data, len, start)` takes
/// the seed as a parameter because different firmware sections use
/// different ones -- the Wi-Fi config block seeds from `0x0000`
/// (`FirmwareHeader::UpdateChecksum` -> `CRC16(&Bytes[0x2C],
/// WifiConfigLength, 0x0000)`), not the `0xFFFF` user-settings use. Getting
/// this wrong produces a checksum a real driver's firmware validation
/// rejects as corrupt, silently disabling Wi-Fi for the rest of the
/// session with no register activity at all -- exactly the symptom this
/// fixes. See `docs/design/design_lan.md` §7.2.
const fn crc16_seeded(data: &[u8], seed: u16) -> u16 {
    let mut crc: u16 = seed;
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i] as u16;
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

/// Length of the synthetic Wi-Fi config block, in bytes, starting at
/// firmware offset `02Ch`. melonDS's generated-firmware fallback (used
/// when no real firmware dump is supplied) declares `0x138`; matching that
/// exactly -- rather than the smaller `0x108` that only covers through the
/// last byte this module actually synthesizes -- means the checksummed
/// region matches what real driver code expects to validate. The extra
/// bytes (Type-3 `Unused0`, `WifiBoard`, `WifiFlash`, reserved) are left
/// zeroed, matching a factory-fresh firmware. See
/// `docs/design/design_lan.md` §7.1-§7.2.
const WIFI_CONFIG_LEN: usize = 0x138;

/// Fills a synthetic **Type-3** Wi-Fi RF/channel calibration block at
/// firmware offset `02Ch`, so [`crate::hw::wifi::Wifi::load_firmware_config`]
/// (in `nds-core`) can resolve a channel from RF register writes instead of
/// leaving every lunaris instance's Wi-Fi permanently unable to select a
/// channel. The RF/BB register *values* below are not from a real
/// firmware dump -- they only need to be distinct and non-zero per channel,
/// which is what channel detection actually depends on
/// (`docs/design/design_lan.md` §3.2 trap 3, §7.2).
///
/// GBATEK "DS Firmware Wifi Calibration Data":
/// <https://problemkaputt.de/gbatek.htm#dsfirmwarewificalibrationdata>
const fn write_wifi_config<const N: usize>(firmware: &mut [u8; N]) {
    // 02Fh: Wi-Fi version (5 = "V6_7", a common real-hardware value).
    firmware[0x02F] = 5;
    // 036h-03Bh: MAC address placeholder. Patched per-instance by
    // `gui/common/src/loader.rs` (`docs/design/design_lan.md` §7.3) so two
    // lunaris processes never boot with identical MACs.
    firmware[0x036] = 0x00;
    firmware[0x037] = 0x09;
    firmware[0x038] = 0xBF;
    firmware[0x039] = 0x00;
    firmware[0x03A] = 0x00;
    firmware[0x03B] = 0x00;
    // 03Ch-03Dh: enabled channels 1-13.
    firmware[0x03C] = 0xFE;
    firmware[0x03D] = 0x3F;
    // 040h: RF chip type = Type-3.
    firmware[0x040] = 3;
    // 041h-042h: RF bits-per-entry / entry count. Matched to melonDS's
    // generated-firmware fallback values (0x94, 0x29) for fidelity; unlike
    // the CRC seed above, real driver code doesn't appear to gate Wi-Fi
    // activation on these specifically, but there's no reason to diverge.
    firmware[0x041] = 0x94;
    firmware[0x042] = 0x29;

    // Type-3 channel table (absolute offsets, see
    // `docs/design/design_lan.md` §7.1): RFIndex1=116h, RFData1[14]
    // =117h..125h, RFIndex2=125h, RFData2[14]=126h..134h. Index values (0,
    // 1) just need to name two distinct RF registers; data values (0x21+i,
    // 0x41+i) just need to be distinct and non-zero per channel.
    firmware[0x116] = 0;
    firmware[0x125] = 1;
    let mut ch = 0_usize;
    while ch < 14 {
        firmware[0x117 + ch] = (0x21 + ch) as u8;
        firmware[0x126 + ch] = (0x41 + ch) as u8;
        ch += 1;
    }

    // 02Ch-02Dh: config length; 02Ah-02Bh: CRC16 over the config block.
    firmware[0x02C] = (WIFI_CONFIG_LEN & 0xFF) as u8;
    firmware[0x02D] = (WIFI_CONFIG_LEN >> 8) as u8;

    let mut buf = [0_u8; WIFI_CONFIG_LEN];
    let mut i = 0;
    while i < WIFI_CONFIG_LEN {
        buf[i] = firmware[0x02C + i];
        i += 1;
    }
    let crc = crc16_seeded(&buf, 0x0000);
    firmware[0x02A] = (crc & 0xFF) as u8;
    firmware[0x02B] = (crc >> 8) as u8;
}

/// Const-compatible firmware creation
const fn default_firmware<const N: usize>(model: DSType) -> [u8; N] {
    let mut firmware = [0_u8; N];

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
    firmware[0x1D] = model.model_spec();

    // 0x1E..0x1F
    firmware[0x1E] = 0xFF;
    firmware[0x1F] = 0xFF;

    write_wifi_config(&mut firmware);

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

        firmware[start] = 5;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for a real Union-Room-never-sees-a-peer bug: the
    /// Wi-Fi config checksum at `02Ah` was computed with the wrong CRC16
    /// seed (`0xFFFF`, copy-pasted from the user-settings checksum,
    /// instead of the `0x0000` melonDS's `FirmwareHeader::UpdateChecksum`
    /// actually uses). A driver's firmware validation would see this as a
    /// corrupt Wi-Fi config and silently disable wireless for the whole
    /// session -- with zero further `W_*` register activity, which is
    /// exactly what was observed. See `docs/design/design_lan.md` §7.2.
    #[test]
    fn wifi_config_checksum_uses_the_zero_seed_not_the_user_settings_seed() {
        let stored = FIRMWARE_DS[0x02A] as u16 | (FIRMWARE_DS[0x02B] as u16) << 8;
        let recomputed = crc16_seeded(&FIRMWARE_DS[0x02C..0x02C + WIFI_CONFIG_LEN], 0x0000);
        assert_eq!(stored, recomputed, "Wi-Fi config CRC must use seed 0x0000, not 0xFFFF");
    }

    #[test]
    fn wifi_config_length_matches_melonds_generated_firmware() {
        let len = FIRMWARE_DS[0x02C] as u16 | (FIRMWARE_DS[0x02D] as u16) << 8;
        assert_eq!(len, 0x138);
    }
}
