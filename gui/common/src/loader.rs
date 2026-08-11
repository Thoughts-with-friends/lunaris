use std::{fs, path::PathBuf};

use nds_core::{NDS, normalize_foreign_save};

use crate::config::Config;

/// Convenience constructor: loads BIOS / firmware / ROM from the filesystem
/// and returns a ready-to-run [`NDS`].
///
/// Falls back to the bundled free BIOS / firmware when paths are `None`.
/// The save file is created automatically next to the ROM if absent.
pub fn load_rom(config: &Config) -> NDS {
    let rom_path = config.last_rom_path.as_ref().unwrap();
    let save_path = create_save_path(config).unwrap();

    // DeSmuME workflow support: if no `.sav` exists yet but a DeSmuME
    // `.dsv` does, adopt it as the starting point for the `.sav` this
    // session writes. The `.dsv` itself is only ever read here, never
    // modified. See `docs/design/ir-nand-foreign-sav-design.md` §3.3.
    if !save_path.exists() {
        let dsv_path = rom_path.with_extension("dsv");
        if let Ok(dsv_bytes) = fs::read(&dsv_path) {
            nds_core::log::info!(
                target: "nds_core::savedata",
                "No .sav found, but found a DeSmuME save at {}; adopting it",
                dsv_path.display()
            );
            let normalized = normalize_foreign_save(&dsv_bytes);
            if let Err(err) = fs::write(&save_path, &normalized) {
                nds_core::log::warn!(
                    target: "nds_core::savedata",
                    "Failed to write .sav from adopted .dsv: {err}"
                );
            }
        } else {
            nds_core::log::info!(
                target: "nds_core::savedata",
                "Save file not found, one will be created on first write at {}",
                save_path.display()
            );
        }
    }

    let bios7 = config
        .bios7_path
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .unwrap_or_else(|| free_bios::arm7::BIOS_ARM7_BIN.to_vec());

    let bios9 = config
        .bios9_path
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .unwrap_or_else(|| free_bios::arm9::BIOS_ARM9_BIN.to_vec());

    // Every firmware image this process actually loads is a per-process
    // working copy, named with this process's id -- never the original
    // configured/default path directly. Two reasons, both hit in practice
    // when testing local multiplayer by launching the same executable
    // twice from one directory (`docs/design/review_mp_local.md`, filed
    // after that setup still failed to associate despite F13):
    //
    // 1. A *shared* filename (the previous `freebios_firmware.bin`, with no
    //    per-process differentiation) races between concurrent processes:
    //    each one truncates and rewrites it independently, so whichever
    //    process reads it last can observe a half-written file or the
    //    other process's MAC patch.
    // 2. Patching a user-configured real firmware dump *in place* would
    //    permanently overwrite its real MAC address on every launch, and
    //    still races the same way if the user points two instances at the
    //    same file.
    let pid = std::process::id();
    let resolved_firmware_path = if let Some(path) = config.firmware_path.as_ref() {
        // Ensure the *source* exists (only fill it in if missing, never
        // overwrite -- it may hold a real dump the user placed there
        // intentionally), then work from a private copy so it is never
        // mutated.
        if !path.exists() {
            fs::write(path, free_bios::firmware::FIRMWARE_DS).unwrap();
        }
        let working_copy = std::env::temp_dir().join(format!("lunaris_firmware_user_{pid}.bin"));
        fs::copy(path, &working_copy).unwrap();
        working_copy
    } else {
        // The default synthetic firmware is regenerated unconditionally,
        // every launch -- it's a pure, instant `const fn` of the embedded
        // `free_bios` crate, so there is nothing to gain by caching it
        // across runs, and caching it is actively harmful: a `free_bios`
        // update (e.g. a Wi-Fi calibration fix) would otherwise sit
        // silently unused behind a stale temp-dir copy from a previous
        // build, exactly as happened while diagnosing
        // `docs/design/design_lan.md`'s Union-Room symptom.
        let default_path = std::env::temp_dir().join(format!("freebios_firmware_{pid}.bin"));
        fs::write(&default_path, free_bios::firmware::FIRMWARE_DS).unwrap();
        default_path
    };

    // Two lunaris instances sharing one firmware image (the common case for
    // local multiplayer, since both usually start from the same bundled
    // synthetic firmware or the same dumped file) would otherwise present
    // identical Wi-Fi MAC addresses. The DS association handshake and every
    // MP frame filter (`Wifi::mac_matches`) are MAC-based, so two identical
    // MACs make association impossible -- see
    // `docs/design/review_mp_local.md` F13 and
    // `docs/design/complete/tmp/design_lan.md` §7.3.
    //
    // `effective_mac_suffix` (not the raw persisted `mac_suffix`) is used
    // deliberately: `config.json` is loaded from the working directory, so
    // two processes launched from the same directory share the same
    // persisted suffix and would otherwise still collide. See
    // `LanConfig::effective_mac_suffix`'s doc comment for the full
    // rationale; whatever this returns must also be what
    // `RoomConfig::mac_suffix` carries, so the room's collision check
    // compares the values actually in use.
    patch_firmware_mac(&resolved_firmware_path, config.lan.effective_mac_suffix());

    let mut nds =
        NDS::new(bios7, bios9, resolved_firmware_path, fs::read(rom_path).unwrap(), save_path);
    nds.set_audio_volume(config.audio_volume);
    nds
}

/// Overwrites the firmware image's Wi-Fi MAC address (header offset
/// `0x036..0x03C`, GBATEK "DS Firmware Header") with `00:09:BF:<suffix>` --
/// melonDS's `DEFAULT_MAC` vendor prefix -- and recomputes the Wi-Fi config
/// block's CRC16 at `0x02A` (`FirmwareHeader::UpdateChecksum` ->
/// `CRC16(&Bytes[0x2C], WifiConfigLength, 0x0000)`), so the ARM7 firmware
/// boot code's own checksum validation still accepts the header. Silently
/// does nothing if the file is missing or too short to contain a firmware
/// header -- a corrupt/foreign file is not this function's problem to
/// diagnose, and `SPI::new` will fail loudly on it regardless.
fn patch_firmware_mac(firmware_path: &std::path::Path, suffix: [u8; 3]) {
    const MAC_OFFSET: usize = 0x036;
    const WIFI_CONFIG_LEN_OFFSET: usize = 0x02C;
    const WIFI_CONFIG_CRC_OFFSET: usize = 0x02A;

    let Ok(mut bytes) = fs::read(firmware_path) else { return };
    if bytes.len() < MAC_OFFSET + 6 || bytes.len() < WIFI_CONFIG_LEN_OFFSET + 2 {
        return;
    }

    bytes[MAC_OFFSET] = 0x00;
    bytes[MAC_OFFSET + 1] = 0x09;
    bytes[MAC_OFFSET + 2] = 0xBF;
    bytes[MAC_OFFSET + 3..MAC_OFFSET + 6].copy_from_slice(&suffix);

    let config_len =
        u16::from_le_bytes([bytes[WIFI_CONFIG_LEN_OFFSET], bytes[WIFI_CONFIG_LEN_OFFSET + 1]])
            as usize;
    let region_end = (WIFI_CONFIG_LEN_OFFSET + config_len).min(bytes.len());
    if region_end > WIFI_CONFIG_LEN_OFFSET {
        let crc = crc16_seeded(&bytes[WIFI_CONFIG_LEN_OFFSET..region_end], 0x0000);
        bytes[WIFI_CONFIG_CRC_OFFSET..WIFI_CONFIG_CRC_OFFSET + 2]
            .copy_from_slice(&crc.to_le_bytes());
    }

    let _ = fs::write(firmware_path, &bytes);
}

/// CRC-16/ARC (poly `0xA001`), matching melonDS's `SPI.cpp::CRC16` and
/// bit-identical to `free_bios::firmware`'s private const-fn twin (verified
/// there against a known-good checksum in
/// `wifi_config_checksum_uses_the_zero_seed_not_the_user_settings_seed`).
/// Kept as a small local copy rather than exposing `free_bios`'s version,
/// since this is the only caller outside that crate.
fn crc16_seeded(data: &[u8], seed: u16) -> u16 {
    let mut crc: u16 = seed;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// <save_dir>/<rom_name>.sav
///
/// # Return None
/// If not exits config.last_rom_path
pub fn create_save_path(config: &Config) -> Option<PathBuf> {
    let rom_path = config.last_rom_path.as_ref()?;
    let rom_stem = rom_path.file_stem()?;
    let save_dir = &config.save_dir;
    let _ = std::fs::create_dir_all(save_dir);
    Some(save_dir.join(format!("{}.sav", rom_stem.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two instances of the same synthetic firmware must end up with
    /// different MAC addresses after patching, and the patched header must
    /// still self-validate: the CRC16 this function writes at `0x02A` must
    /// match an independent recomputation over the same region, exactly as
    /// a real ARM7 firmware boot would check it.
    #[test]
    fn patched_firmware_gets_a_distinct_valid_mac() {
        let dir =
            std::env::temp_dir().join(format!("lunaris_mac_patch_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path_a = dir.join("fw_a.bin");
        let path_b = dir.join("fw_b.bin");
        std::fs::write(&path_a, &free_bios::firmware::FIRMWARE_DS[..]).unwrap();
        std::fs::write(&path_b, &free_bios::firmware::FIRMWARE_DS[..]).unwrap();

        patch_firmware_mac(&path_a, [0x11, 0x22, 0x33]);
        patch_firmware_mac(&path_b, [0xAA, 0xBB, 0xCC]);

        let bytes_a = std::fs::read(&path_a).unwrap();
        let bytes_b = std::fs::read(&path_b).unwrap();

        assert_eq!(&bytes_a[0x036..0x03C], &[0x00, 0x09, 0xBF, 0x11, 0x22, 0x33]);
        assert_eq!(&bytes_b[0x036..0x03C], &[0x00, 0x09, 0xBF, 0xAA, 0xBB, 0xCC]);
        assert_ne!(&bytes_a[0x036..0x03C], &bytes_b[0x036..0x03C]);

        for bytes in [&bytes_a, &bytes_b] {
            let config_len = u16::from_le_bytes([bytes[0x02C], bytes[0x02D]]) as usize;
            let stored_crc = u16::from_le_bytes([bytes[0x02A], bytes[0x02B]]);
            let recomputed = crc16_seeded(&bytes[0x02C..0x02C + config_len], 0x0000);
            assert_eq!(stored_crc, recomputed, "Wi-Fi config CRC16 must validate after the patch");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
