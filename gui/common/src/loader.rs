//! Boots a [`Console`] from the front end's [`Config`].
//!
//! ```text
//!  Config (ROM, save dir, BIOS / firmware paths, LAN MAC suffix, volume)
//!    |  load_rom_for_instance(config, instance)
//!    |    1. no .sav yet but a DeSmuME .dsv: adopt it as the .sav
//!    |    2. pick the Wi-Fi MAC (unique per process and instance)
//!    v
//!  Console::new(Boot)  -> ds_emu loads BIOS, firmware, ROM and .sav
//! ```
use std::{fs, path::PathBuf};

use ds_emu::normalize_foreign_save;

use crate::{
    config::Config,
    console::{Boot, Console},
};

/// Boots the game in `config.last_rom_path` as instance 0. A ROM that cannot
/// be booted is logged and gives [`Console::placeholder`].
pub fn load_rom(config: &Config) -> Console {
    load_rom_for_instance(config, 0)
}

/// A console without a game, for front ends that start on the ROM picker.
/// Emulating it does nothing.
pub fn placeholder_nds() -> Console {
    Console::placeholder()
}

/// Like [`load_rom`], for one of several consoles in the **same process**
/// (Thread Mode). `instance` must be unique per live console: it picks the
/// audio stream, and it makes the Wi-Fi MAC unique (two DS radios sharing a
/// MAC can never complete 802.11 authentication with each other, so local
/// play would loop on auth forever).
pub fn load_rom_for_instance(config: &Config, instance: u8) -> Console {
    match try_load_rom_for_instance(config, instance) {
        Ok(console) => console,
        Err(err) => {
            log::error!(target: "ds_cartridge", "Cannot boot the ROM: {err}");
            Console::placeholder()
        }
    }
}

/// [`load_rom_for_instance`] that reports why a ROM could not be booted.
///
/// # Errors
/// If no ROM is set, or it or its save file cannot be read.
pub fn try_load_rom_for_instance(config: &Config, instance: u8) -> Result<Console, String> {
    let rom_path = config.last_rom_path.as_ref().ok_or("no ROM selected")?;
    let save_path = save_path_for_instance(config, instance).ok_or("the ROM path has no name")?;
    adopt_desmume_save(rom_path, &save_path);

    let mac = wifi_mac(config, instance);
    Console::new(&Boot {
        rom: rom_path,
        save_dir: &config.save_dir,
        bios7: config.bios7_path.as_deref(),
        bios9: config.bios9_path.as_deref(),
        firmware: config.firmware_path.as_deref(),
        mac,
        audio_instance: Some(instance),
        volume: config.audio_volume,
    })
}

/// DeSmuME workflow support: with no `.sav` yet but a `.dsv` beside the ROM,
/// the `.dsv` (footer stripped) becomes the `.sav`. The `.dsv` itself is
/// only read.
fn adopt_desmume_save(rom_path: &std::path::Path, save_path: &std::path::Path) {
    if save_path.exists() {
        return;
    }
    let dsv_path = rom_path.with_extension("dsv");
    if let Ok(dsv_bytes) = fs::read(&dsv_path) {
        log::info!(
            target: "ds_cartridge",
            "No .sav found, but found a DeSmuME save at {}; adopting it",
            dsv_path.display()
        );
        if let Err(err) = fs::write(save_path, normalize_foreign_save(&dsv_bytes)) {
            log::warn!(target: "ds_cartridge", "Failed to write .sav from adopted .dsv: {err}");
        }
    } else {
        log::info!(
            target: "ds_cartridge",
            "Save file not found, one will be created on first write at {}",
            save_path.display()
        );
    }
}

/// The Wi-Fi MAC of instance `instance` (`None`: keep the firmware's).
///
/// A user firmware dump keeps its MAC on instance 0 and gets a perturbed one
/// on the others (melonDS `EmuInstance.cpp`). The built-in firmware gets the
/// configured LAN suffix mixed with the process id and the instance, so two
/// lunaris processes on one machine (same config file) still differ.
fn wifi_mac(config: &Config, instance: u8) -> Option<[u8; 6]> {
    let dump = config.firmware_path.as_ref().and_then(|p| fs::read(p).ok());
    if let Some(dump) = dump.filter(|d| d.len() >= 0x3C) {
        if instance == 0 {
            return None;
        }
        let mut mac = [0; 6];
        mac.copy_from_slice(&dump[0x36..0x3C]);
        return Some(perturb_mac(mac, instance));
    }
    let pid = std::process::id();
    let s = config.lan.mac_suffix;
    let oui = &ds_bios::firmware::FIRMWARE_DS[0x36..0x39];
    Some([
        oui[0],
        oui[1],
        oui[2],
        s[0] ^ (pid & 0xFF) as u8 ^ instance,
        s[1] ^ ((pid >> 8) & 0xFF) as u8 ^ instance.wrapping_mul(0x44),
        s[2] ^ ((pid >> 16) & 0xFF) as u8 ^ instance.wrapping_mul(0x10),
    ])
}

/// melonDS's per-instance MAC: add the instance to byte 3, `* 0x44` to
/// byte 4 and `* 0x10` to byte 5, and clear the multicast / locally
/// administered bits. Instance 0 is unchanged.
fn perturb_mac(mut mac: [u8; 6], instance: u8) -> [u8; 6] {
    if instance == 0 {
        return mac;
    }
    mac[3] = mac[3].wrapping_add(instance);
    mac[4] = mac[4].wrapping_add(instance.wrapping_mul(0x44));
    mac[5] = mac[5].wrapping_add(instance.wrapping_mul(0x10));
    mac[0] &= 0xFC;
    mac
}

/// The `.sav` path instance `instance` writes.
///
/// Separation now comes from `config.save_dir`, which
/// [`Config::load_for_instance`] points at `instances/instance<N>/saves`, so the
/// filename itself needs no instance marker. Callers that act on a running
/// instance's save (importing, exporting) use this to get the same answer
/// [`load_rom_for_instance`] used to boot it.
///
/// `config` must be the instance's own configuration. Passing another
/// instance's would aim this at that instance's save file — which is why
/// [`Config::load_for_instance`] rewrites the directories on every load rather
/// than trusting what the JSON happens to contain.
#[must_use]
pub fn save_path_for_instance(config: &Config, _instance: u8) -> Option<PathBuf> {
    create_save_path(config)
}

/// The savestate directory instance `instance` uses.
///
/// As with [`save_path_for_instance`], the separation lives in
/// `config.save_state_dir`. Slot numbering is therefore per instance already:
/// the guest's "State 1" cannot overwrite the host's, since the two run
/// independent timelines of the same ROM.
#[must_use]
pub fn state_dir_for_instance(config: &Config, _instance: u8) -> Option<PathBuf> {
    let rom_stem = config.last_rom_path.as_ref()?.file_stem()?;
    Some(config.save_state_dir.join(rom_stem))
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

    /// Two Thread Mode instances must not share a MAC, and the result is
    /// never multicast / locally administered.
    #[test]
    fn per_instance_mac_differs() {
        let base = [0x00, 0x09, 0xBF, 0x12, 0x34, 0x56];
        let guest = perturb_mac(base, 1);
        assert_ne!(guest, base, "instance 1 needs its own MAC");
        assert_eq!(guest[0] & 0x03, 0);
        assert_eq!(perturb_mac(base, 0), base);

        let config = Config::default();
        assert_ne!(wifi_mac(&config, 0), wifi_mac(&config, 1));
    }

    /// Two instances must never write one another's `.sav` or savestate slots.
    #[test]
    fn instances_get_separate_save_and_state_paths() {
        let instance_config = |instance: u8| {
            let mut config = Config {
                last_rom_path: Some(PathBuf::from("/roms/game.nds")),
                ..Config::default()
            };
            let dir = Config::instance_dir(instance);
            config.save_dir = dir.join("saves");
            config.save_state_dir = dir.join("states");
            config
        };
        let (host, guest) = (instance_config(0), instance_config(1));

        assert_ne!(
            save_path_for_instance(&host, 0).unwrap(),
            save_path_for_instance(&guest, 1).unwrap(),
        );
        assert_ne!(
            state_dir_for_instance(&host, 0).unwrap(),
            state_dir_for_instance(&guest, 1).unwrap(),
        );
    }

    /// Instances are numbered from 1 on disk.
    #[test]
    fn instance_directories_are_numbered_from_one() {
        assert!(Config::instance_dir(0).ends_with("instance1"));
        assert!(Config::instance_dir(1).ends_with("instance2"));
        assert!(Config::instance_config_path(1).ends_with("instance2/config.json"));
    }

    /// Boots `LUNARIS_TEST_ROM` the way the egui GUI does, runs it, and
    /// round-trips a savestate file.
    #[test]
    #[ignore = "requires a ROM (LUNARIS_TEST_ROM)"]
    fn boots_a_real_rom_and_round_trips_a_state() {
        let rom = PathBuf::from(std::env::var("LUNARIS_TEST_ROM").expect("LUNARIS_TEST_ROM"));
        let dir = std::env::temp_dir().join(format!("lunaris_loader_{}", std::process::id()));
        let config =
            Config { last_rom_path: Some(rom), save_dir: dir.join("saves"), ..Config::default() };
        let mut console = try_load_rom_for_instance(&config, 7).expect("boots");
        for _ in 0..120 {
            console.emulate_frame();
        }
        assert!(console.get_screens().iter().any(|s| s.iter().any(|&px| px & 0xFF_FFFF != 0)));

        let state = dir.join("state_1.bin");
        crate::savestate::save_to_file(&console, &state).expect("saves");
        let before: Vec<u32> = console.get_screens()[0].to_vec();
        for _ in 0..30 {
            console.emulate_frame();
        }
        crate::savestate::load_from_file(&mut console, &state).expect("loads");
        assert_eq!(console.get_screens()[0], before.as_slice());
        console.emulate_frame();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn placeholder_has_no_rom() {
        assert!(!placeholder_nds().has_rom());
    }
}
