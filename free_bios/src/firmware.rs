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
/// Baseband initialisation table, uploaded by the driver during Wi-Fi init.
/// Copied verbatim from melonDS's generated firmware (`BBINIT`,
/// `docs/design/melonds/SPI_Firmware.cpp:30-38`). Lives at firmware `064h`
/// (`InitialBBValues[105]`).
const BB_INIT: [u8; 0x69] = [
    0x03, 0x17, 0x40, 0x00, 0x1B, 0x6C, 0x48, 0x80, 0x38, 0x00, 0x35, 0x07, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0xBB, 0x01, 0x24, 0x7F,
    0x5A, 0x01, 0x3F, 0x01, 0x3F, 0x36, 0x1D, 0x00, 0x78, 0x35, 0x55, 0x12, 0x34, 0x1C, 0x00, 0x01,
    0x0E, 0x38, 0x03, 0x70, 0xC5, 0x2A, 0x0A, 0x08, 0x04, 0x01, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFE,
    0xFE, 0xFE, 0xFE, 0xFC, 0xFC, 0xFA, 0xFA, 0xFA, 0xFA, 0xFA, 0xF8, 0xF8, 0xF6, 0x00, 0x12, 0x14,
    0x12, 0x41, 0x23, 0x03, 0x04, 0x70, 0x35, 0x0E, 0x2C, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x0E, 0x00, 0x00, 0x12, 0x28, 0x1C,
];

/// RF initialisation table (Type-3), uploaded to RF registers `00h`-`28h`
/// during Wi-Fi init. Copied verbatim from melonDS's `RFINIT`
/// (`docs/design/melonds/SPI_Firmware.cpp:41-46`). Lives at firmware `0CEh`
/// (`Type3Config.InitialRFValues[41]`).
///
/// Leaving this zeroed -- as this firmware used to -- makes the driver
/// upload zeros across the whole RF register file, including the two
/// channel-selection registers, so the emulated radio never holds a value
/// that [`nds_core`]'s channel detection can match.
const RF_INIT: [u8; 0x29] = [
    0x31, 0x4C, 0x4F, 0x21, 0x00, 0x10, 0xB0, 0x08, 0xFA, 0x15, 0x26, 0xE6, 0xC1, 0x01, 0x0E, 0x50,
    0x05, 0x00, 0x6D, 0x12, 0x00, 0x00, 0x01, 0xFF, 0x0E, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x06,
    0x06, 0x00, 0x00, 0x00, 0x18, 0x00, 0x02, 0x00, 0x00,
];

/// Per-channel BB/RF calibration, copied verbatim from melonDS's `CHANDATA`
/// (`docs/design/melonds/SPI_Firmware.cpp:48-55`) and placed at firmware
/// `0F8h`, exactly as melonDS does (`SPI_Firmware.cpp:147`).
///
/// Layout, 60 bytes: `BBIndex1`, `BBData1[14]`, `BBIndex2`, `BBData2[14]`,
/// `RFIndex1`, `RFData1[14]`, `RFIndex2`, `RFData2[14]`. The two RF index
/// bytes are `01h` and `02h`, i.e. the driver selects a channel by writing
/// `RFData1[ch]` to RF register 1 and `RFData2[ch]` to RF register 2 --
/// which is what `Wifi::change_channel` matches against.
const CHAN_DATA: [u8; 0x3C] = [
    0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x16, 0x26,
    0x1C, 0x1C, 0x1C, 0x1D, 0x1D, 0x1D, 0x1E, 0x1E, 0x1E, 0x1E, 0x1F, 0x1E, 0x1F, 0x18, 0x01, 0x4B,
    0x4B, 0x4B, 0x4B, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4D, 0x4D, 0x4D, 0x02, 0x6C, 0x71,
    0x76, 0x5B, 0x40, 0x45, 0x4A, 0x2F, 0x34, 0x39, 0x3E, 0x03, 0x08, 0x14,
];

/// The 16 halfwords melonDS writes at firmware `044h`-`063h`
/// (`SPI_Firmware.cpp:127-142`), between `Unknown3` and `InitialBBValues`.
const INITIAL_VALUES: [u16; 16] = [
    0x0002, 0x0017, 0x0026, 0x1818, 0x0048, 0x4840, 0x0058, 0x0042, 0x0146, 0x8064, 0xE6E6, 0x2443,
    0x000E, 0x0001, 0x0001, 0x0402,
];

const fn write_wifi_config<const N: usize>(firmware: &mut [u8; N]) {
    // 02Fh: Wi-Fi version. melonDS's generated DS firmware reports W006
    // (`SPI_Firmware.cpp:111`); this used to say 5, which is a DSi-era value.
    firmware[0x02F] = 6;
    // 030h-035h: Unused3, which melonDS fills with a fixed pattern.
    firmware[0x030] = 0xFF;
    firmware[0x031] = 0xFF;
    firmware[0x032] = 0xFF;
    firmware[0x033] = 0xFF;
    firmware[0x034] = 0xFF;
    firmware[0x035] = 0x00;
    // 036h-03Bh: MAC address placeholder. Patched per-instance by
    // `gui/common/src/loader.rs` (`docs/design/design_lan.md` §7.3) so two
    // lunaris processes never boot with identical MACs.
    firmware[0x036] = 0x00;
    firmware[0x037] = 0x09;
    firmware[0x038] = 0xBF;
    firmware[0x039] = 0x00;
    firmware[0x03A] = 0x00;
    firmware[0x03B] = 0x00;
    // 03Ch-03Dh: enabled channels 1-13 (0x3FFE).
    firmware[0x03C] = 0xFE;
    firmware[0x03D] = 0x3F;
    // 03Eh-03Fh: Unknown2.
    firmware[0x03E] = 0xFF;
    firmware[0x03F] = 0xFF;
    // 040h: RF chip type = Type-3.
    firmware[0x040] = 3;
    // 041h-043h: RF bits-per-entry, entry count, Unknown3.
    firmware[0x041] = 0x94;
    firmware[0x042] = 0x29;
    firmware[0x043] = 0x02;

    // 044h-063h: sixteen halfwords melonDS plants verbatim.
    let mut i = 0;
    while i < 16 {
        let v = INITIAL_VALUES[i];
        firmware[0x044 + i * 2] = (v & 0xFF) as u8;
        firmware[0x045 + i * 2] = (v >> 8) as u8;
        i += 1;
    }

    // 064h-0CCh: InitialBBValues.
    let mut i = 0;
    while i < BB_INIT.len() {
        firmware[0x064 + i] = BB_INIT[i];
        i += 1;
    }
    // 0CDh: Unused4 stays zero.

    // 0CEh-0F6h: Type-3 InitialRFValues.
    let mut i = 0;
    while i < RF_INIT.len() {
        firmware[0x0CE + i] = RF_INIT[i];
        i += 1;
    }
    // 0F7h: BBIndicesPerChannel.
    firmware[0x0F7] = 0x02;

    // 0F8h-133h: per-channel BB/RF table (BBIndex1/BBData1/BBIndex2/BBData2/
    // RFIndex1/RFData1/RFIndex2/RFData2).
    let mut i = 0;
    while i < CHAN_DATA.len() {
        firmware[0x0F8 + i] = CHAN_DATA[i];
        i += 1;
    }

    // 134h-161h: Type3Config.Unused0, which melonDS fills with 0xFF.
    let mut i = 0;
    while i < 46 {
        firmware[0x134 + i] = 0xFF;
        i += 1;
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

#[cfg(test)]
mod wifi_calibration_tests {
    use super::*;

    /// The synthetic firmware must carry a real, self-consistent Wi-Fi
    /// calibration block, not placeholder values. The DS driver uploads
    /// `InitialRFValues` across the whole RF register file during init; if
    /// those are zeros it clobbers the two channel-selection registers, and
    /// no channel can ever be detected -- which is what made local wireless
    /// play impossible.
    #[test]
    fn wifi_config_carries_melonds_calibration_tables() {
        let fw = &FIRMWARE_DS;

        assert_eq!(fw[0x040], 3, "RFChipType must be Type-3");
        assert_eq!(fw[0x041], 0x94, "RFBitsPerEntry");
        assert_eq!(fw[0x042], 0x29, "RFEntries");

        // InitialBBValues / InitialRFValues must not be blank.
        assert_eq!(&fw[0x064..0x064 + BB_INIT.len()], &BB_INIT[..]);
        assert_eq!(&fw[0x0CE..0x0CE + RF_INIT.len()], &RF_INIT[..]);
        assert!(RF_INIT.iter().any(|&b| b != 0), "RF init table must not be all zeros");

        // Per-channel table at 0F8h, and the two RF index bytes the driver
        // uses to select a channel.
        assert_eq!(&fw[0x0F8..0x0F8 + CHAN_DATA.len()], &CHAN_DATA[..]);
        assert_eq!(fw[0x116], 0x01, "RFIndex1");
        assert_eq!(fw[0x125], 0x02, "RFIndex2");
    }

    /// Every channel's `(RFData1, RFData2)` pair must be distinct, or
    /// `Wifi::change_channel` cannot tell channels apart. melonDS's table
    /// satisfies this; a hand-written placeholder easily does not.
    #[test]
    fn every_channel_has_a_distinct_rf_value_pair() {
        let fw = &FIRMWARE_DS;
        let pairs: Vec<(u8, u8)> = (0..14).map(|i| (fw[0x117 + i], fw[0x126 + i])).collect();
        for (i, a) in pairs.iter().enumerate() {
            for (j, b) in pairs.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "channels {} and {} share an RF value pair {a:?}", i + 1, j + 1);
            }
        }
    }
}

/// Returns the synthetic DS firmware with its Wi-Fi MAC address set to
/// `00:09:BF:xx:yy:zz`, where the last three bytes come from `suffix`, and
/// the Wi-Fi config checksum recomputed to match.
///
/// Every `lunaris` process must boot with a **distinct** MAC. Two instances
/// sharing one address cannot complete 802.11 authentication with each
/// other: each sees frames that appear to come from itself, so the
/// authentication exchange repeats forever and association never starts --
/// which is exactly what local wireless play between two default-configured
/// instances used to do.
///
/// The `00:09:BF` prefix is Nintendo's OUI and is kept; only the
/// device-specific half varies. The checksum at `02Ah` covers the whole
/// Wi-Fi config block from `02Ch`, which includes the MAC at `036h`, so it
/// has to be recomputed rather than left alone.
#[must_use]
pub fn firmware_ds_with_mac_suffix(suffix: [u8; 3]) -> Vec<u8> {
    let mut fw = FIRMWARE_DS.to_vec();
    fw[0x036] = 0x00;
    fw[0x037] = 0x09;
    fw[0x038] = 0xBF;
    fw[0x039] = suffix[0];
    fw[0x03A] = suffix[1];
    fw[0x03B] = suffix[2];

    let crc = crc16_seeded(&fw[0x02C..0x02C + WIFI_CONFIG_LEN], 0x0000);
    fw[0x02A] = (crc & 0xFF) as u8;
    fw[0x02B] = (crc >> 8) as u8;
    fw
}

#[cfg(test)]
mod mac_suffix_tests {
    use super::*;

    /// Patching the MAC must keep Nintendo's OUI, apply the suffix, and
    /// leave the Wi-Fi config checksum valid -- the checksum covers the MAC,
    /// so a naive byte poke would invalidate it.
    #[test]
    fn mac_suffix_is_applied_and_checksum_stays_valid() {
        let fw = firmware_ds_with_mac_suffix([0xAA, 0xBB, 0xCC]);

        assert_eq!(&fw[0x036..0x03C], &[0x00, 0x09, 0xBF, 0xAA, 0xBB, 0xCC]);

        let stored = u16::from(fw[0x02A]) | (u16::from(fw[0x02B]) << 8);
        let recomputed = crc16_seeded(&fw[0x02C..0x02C + WIFI_CONFIG_LEN], 0x0000);
        assert_eq!(stored, recomputed, "Wi-Fi config checksum must match the patched MAC");
    }

    /// Two different suffixes must produce two different MACs -- the whole
    /// point is that two instances never share an address.
    #[test]
    fn different_suffixes_give_different_macs() {
        let a = firmware_ds_with_mac_suffix([1, 2, 3]);
        let b = firmware_ds_with_mac_suffix([4, 5, 6]);
        assert_ne!(&a[0x036..0x03C], &b[0x036..0x03C]);
    }
}
