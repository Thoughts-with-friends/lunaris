use std::{fs, path::PathBuf};

use nds_core::{NDS, normalize_foreign_save};

use crate::config::Config;

/// Convenience constructor: loads BIOS / firmware / ROM from the filesystem
/// and returns a ready-to-run [`NDS`].
///
/// Falls back to the bundled free BIOS / firmware when paths are `None`.
/// The save file is created automatically next to the ROM if absent.
pub fn load_rom(config: &Config) -> NDS {
    load_rom_for_instance(config, 0)
}

/// Like [`load_rom`], but for one of several [`NDS`] instances living in the
/// **same process** (Thread Mode).
///
/// `instance` must be unique per live instance. It matters for two reasons:
///
/// * **MAC address.** The per-process MAC derivation below mixes in the process
///   id, which is by definition identical for every instance in one process, so
///   two of them would boot with the same Wi-Fi MAC. Two DS radios sharing a MAC
///   can never complete 802.11 authentication with each other, so local wireless
///   play would loop on auth forever without ever associating.
/// * **Save file.** Instances past the first get their own `.sav`, so a
///   verification run cannot have two emulators writing one file.
///
/// Instance `0` behaves exactly as [`load_rom`] always has, so the ordinary
/// single-instance path is byte-for-byte unchanged.
pub fn load_rom_for_instance(config: &Config, instance: u8) -> NDS {
    let rom_path = config.last_rom_path.as_ref().unwrap();
    let save_path = save_path_for_instance(config, instance).unwrap();

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

    let resolved_firmware_path = if let Some(path) = config.firmware_path.as_ref() {
        // A user-configured path may hold a real firmware dump they placed
        // there intentionally (touch calibration, console id, a real Wi-Fi
        // config block); only fill it in if it's missing, never overwrite.
        if !path.exists() {
            fs::write(path, free_bios::firmware::FIRMWARE_DS).unwrap();
        }
        if instance == 0 {
            path.to_path_buf()
        } else {
            // A dump is one console's image, so every instance booting it would
            // share its MAC and none of them could associate with the others.
            // Patch a per-instance copy instead -- never the user's file.
            let patched = std::env::temp_dir()
                .join(format!("lunaris_firmware_{}_{instance}.bin", std::process::id()));
            match fs::read(path) {
                Ok(image) => {
                    fs::write(&patched, perturb_firmware_mac(image, instance)).unwrap();
                    patched
                }
                Err(_) => path.to_path_buf(),
            }
        }
    } else {
        // The default synthetic firmware is regenerated unconditionally,
        // every launch -- it's a pure, instant `const fn` of the embedded
        // `free_bios` crate, so there is nothing to gain by caching it
        // across runs, and caching it is actively harmful: a `free_bios`
        // update (e.g. a Wi-Fi calibration fix) would otherwise sit
        // silently unused behind a stale temp-dir copy from a previous
        // build, exactly as happened while diagnosing
        // `docs/design/design_lan.md`'s Union-Room symptom.
        // Per-process path *and* per-process MAC. Two `lunaris` instances on
        // one machine read the same config file, so `config.lan.mac_suffix`
        // alone would give them the same address -- and two DS radios that
        // share a MAC can never complete 802.11 authentication with each
        // other, so local wireless play would loop on auth forever without
        // ever associating. Mixing the process id in guarantees they differ;
        // a separate file keeps them from overwriting each other's copy.
        let pid = std::process::id();
        let cfg_suffix = config.lan.mac_suffix;
        // `instance` is mixed in alongside the process id because Thread Mode
        // runs several instances inside one process, where the pid is identical
        // for all of them. melonDS perturbs the MAC per instance for exactly
        // this reason (`EmuInstance.cpp:1751-1765`). The three bytes are
        // perturbed differently so two instances never collide by cancelling
        // out in a single byte.
        let suffix = [
            cfg_suffix[0] ^ (pid & 0xFF) as u8 ^ instance,
            cfg_suffix[1] ^ ((pid >> 8) & 0xFF) as u8 ^ instance.wrapping_mul(0x44),
            // Bit 0 of the first octet is the multicast flag and bit 1 the
            // locally-administered flag; those live in byte `036h`, which is
            // fixed at `00`, so the suffix bytes are free-form.
            cfg_suffix[2] ^ ((pid >> 16) & 0xFF) as u8 ^ instance.wrapping_mul(0x10),
        ];
        let default_path =
            std::env::temp_dir().join(format!("freebios_firmware_{pid}_{instance}.bin"));
        fs::write(&default_path, free_bios::firmware::firmware_ds_with_mac_suffix(suffix)).unwrap();
        default_path
    };

    let mut nds =
        NDS::new(bios7, bios9, resolved_firmware_path, fs::read(rom_path).unwrap(), save_path);
    nds.set_audio_volume(config.audio_volume);
    nds
}

/// Perturbs a firmware image's MAC address (header `036h`) for instance
/// `instance`, and repairs the Wi-Fi config checksum the MAC sits inside.
///
/// Follows melonDS's per-instance derivation
/// (`docs/design/melonds/frontend/qt_sdl/EmuInstance.cpp:1751-1765`): add the
/// instance index to byte 3, `index * 0x44` to byte 4 and `index * 0x10` to
/// byte 5, then clear the low two bits of byte 0 so the result can never be a
/// multicast or locally-administered address. Instance `0` is returned
/// untouched, matching melonDS's `instanceID > 0` guard.
///
/// The Wi-Fi config block runs from `02Ch` for the length stored at
/// `02Ch..02Eh`, and its CRC16 (seed `0000h`) lives at `02Ah`. The MAC is
/// inside that range, so poking the bytes without recomputing the checksum
/// leaves an image a driver's firmware validation rejects as corrupt --
/// disabling Wi-Fi outright, with no register activity to diagnose from.
fn perturb_firmware_mac(mut image: Vec<u8>, instance: u8) -> Vec<u8> {
    if instance == 0 || image.len() < 0x03C {
        return image;
    }
    image[0x039] = image[0x039].wrapping_add(instance);
    image[0x03A] = image[0x03A].wrapping_add(instance.wrapping_mul(0x44));
    image[0x03B] = image[0x03B].wrapping_add(instance.wrapping_mul(0x10));
    image[0x036] &= 0xFC;

    let len = u16::from_le_bytes([image[0x02C], image[0x02D]]) as usize;
    if let Some(block) = image.get(0x02C..0x02C + len) {
        let crc = crc16(block, 0x0000);
        image[0x02A] = crc as u8;
        image[0x02B] = (crc >> 8) as u8;
    }
    image
}

/// CRC-16/ARC (poly `A001h`) with an explicit seed, as used throughout DS
/// firmware. Mirrors `free_bios::firmware`'s `crc16_seeded`, which is private
/// to that crate.
fn crc16(data: &[u8], seed: u16) -> u16 {
    data.iter().fold(seed, |crc, &byte| {
        (0..8).fold(crc ^ u16::from(byte), |crc, _| {
            if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 }
        })
    })
}

/// The `.sav` path instance `instance` writes.
///
/// Instance `0` uses [`create_save_path`] unchanged; later instances get a
/// `.inst<n>.sav` sibling so two emulators in one process never write one file.
/// Exposed because callers that act on a running instance's save (importing,
/// exporting) need the same answer [`load_rom_for_instance`] used to boot it.
#[must_use]
pub fn save_path_for_instance(config: &Config, instance: u8) -> Option<PathBuf> {
    let base = create_save_path(config)?;
    if instance == 0 {
        return Some(base);
    }
    let stem = base.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into());
    Some(base.with_file_name(format!("{stem}.inst{instance}.sav")))
}

/// The savestate directory instance `instance` uses.
///
/// Slot numbering is per instance: the guest's "State 1" must not overwrite the
/// host's, since the two are running independent timelines of the same ROM.
#[must_use]
pub fn state_dir_for_instance(config: &Config, instance: u8) -> Option<PathBuf> {
    let rom_stem = config.last_rom_path.as_ref()?.file_stem()?;
    let dir = config.save_state_dir.join(rom_stem);
    Some(if instance == 0 { dir } else { dir.join(format!("inst{instance}")) })
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

    /// The local CRC16 must agree with the one `free_bios` used to stamp the
    /// image, or every patched firmware would be rejected as corrupt. Checked
    /// against the untouched synthetic image, whose stored checksum is known
    /// good.
    #[test]
    fn crc16_reproduces_the_stored_wifi_config_checksum() {
        let fw = &free_bios::firmware::FIRMWARE_DS;
        let len = u16::from_le_bytes([fw[0x02C], fw[0x02D]]) as usize;

        let computed = crc16(&fw[0x02C..0x02C + len], 0x0000);

        assert_eq!(computed.to_le_bytes(), [fw[0x02A], fw[0x02B]]);
    }

    /// Two Thread Mode instances must not share a MAC, and patching it has to
    /// leave the Wi-Fi config checksum valid -- the MAC sits inside the
    /// checksummed block, so a naive byte poke produces an image a driver's
    /// firmware validation rejects, silently disabling Wi-Fi.
    #[test]
    fn per_instance_mac_differs_and_keeps_the_checksum_valid() {
        let base = free_bios::firmware::FIRMWARE_DS.to_vec();

        let guest = perturb_firmware_mac(base.clone(), 1);

        assert_ne!(&guest[0x036..0x03C], &base[0x036..0x03C], "instance 1 needs its own MAC");
        assert_eq!(guest[0x036] & 0x03, 0, "never multicast or locally administered");

        let len = u16::from_le_bytes([guest[0x02C], guest[0x02D]]) as usize;
        let crc = crc16(&guest[0x02C..0x02C + len], 0x0000);
        assert_eq!(crc.to_le_bytes(), [guest[0x02A], guest[0x02B]], "checksum must be repaired");
    }

    /// Two instances must never write one another's `.sav` or savestate slots:
    /// they run independent timelines of the same ROM, so a shared path means
    /// one silently destroys the other's progress.
    #[test]
    fn instances_get_separate_save_and_state_paths() {
        let config =
            Config { last_rom_path: Some(PathBuf::from("/roms/game.nds")), ..Config::default() };

        let host_sav = save_path_for_instance(&config, 0).unwrap();
        let guest_sav = save_path_for_instance(&config, 1).unwrap();
        assert_ne!(host_sav, guest_sav);
        assert_eq!(host_sav, create_save_path(&config).unwrap(), "instance 0 is unchanged");

        let host_states = state_dir_for_instance(&config, 0).unwrap();
        let guest_states = state_dir_for_instance(&config, 1).unwrap();
        assert_ne!(host_states, guest_states);
        assert!(guest_states.starts_with(&host_states), "the guest nests under the ROM's dir");
    }

    /// Instance 0 is the main window's emulator and must boot the image exactly
    /// as a single-instance session always has.
    #[test]
    fn instance_zero_is_left_untouched() {
        let base = free_bios::firmware::FIRMWARE_DS.to_vec();
        assert_eq!(perturb_firmware_mac(base.clone(), 0), base);
    }
}
