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

    let resolved_firmware_path = if let Some(path) = config.firmware_path.as_ref() {
        // A user-configured path may hold a real firmware dump they placed
        // there intentionally (touch calibration, console id, a real Wi-Fi
        // config block); only fill it in if it's missing, never overwrite.
        if !path.exists() {
            fs::write(path, free_bios::firmware::FIRMWARE_DS).unwrap();
        }
        path.to_path_buf()
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
        let suffix = [
            cfg_suffix[0] ^ (pid & 0xFF) as u8,
            cfg_suffix[1] ^ ((pid >> 8) & 0xFF) as u8,
            // Bit 0 of the first octet is the multicast flag and bit 1 the
            // locally-administered flag; those live in byte `036h`, which is
            // fixed at `00`, so the suffix bytes are free-form.
            cfg_suffix[2] ^ ((pid >> 16) & 0xFF) as u8,
        ];
        let default_path = std::env::temp_dir().join(format!("freebios_firmware_{pid}.bin"));
        fs::write(&default_path, free_bios::firmware::firmware_ds_with_mac_suffix(suffix)).unwrap();
        default_path
    };

    let mut nds =
        NDS::new(bios7, bios9, resolved_firmware_path, fs::read(rom_path).unwrap(), save_path);
    nds.set_audio_volume(config.audio_volume);
    nds
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
