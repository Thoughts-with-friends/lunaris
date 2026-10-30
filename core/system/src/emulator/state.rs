//! Savestates: the whole console as bytes.
//!
//! ```text
//!  save_state()  Emulator --serde--> bincode bytes
//!  load_state()  bytes --serde--> new Emulator
//!                  + host data moved over from the running one
//!                  (config, BIOS, ROM, .sav path, cheats, Wi-Fi link)
//! ```
//!
//! Host data and render caches are `#[serde(skip)]` throughout the core
//! crates: the ROM is never in a state (compare [`RomFingerprint`]s before
//! loading one), and the 3D rasterizer's buffers are rebuilt by the next
//! frame. The encoding is not stable across core changes; front ends add
//! their own header (format version, ROM fingerprint, compression).
use core::fmt;

use ds_cartridge::RomFingerprint;

use super::Emulator;

/// Why a state could not be made or applied.
#[derive(Debug)]
pub enum StateError {
    Encode(bincode::error::EncodeError),
    Decode(bincode::error::DecodeError),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "cannot encode the savestate: {e}"),
            Self::Decode(e) => write!(f, "cannot decode the savestate: {e}"),
        }
    }
}

impl std::error::Error for StateError {}

const CONFIG: bincode::config::Configuration = bincode::config::standard();

impl Emulator {
    /// The console's state (without the ROM, BIOS and settings).
    ///
    /// # Errors
    /// If a field cannot be encoded.
    pub fn save_state(&self) -> Result<Vec<u8>, StateError> {
        bincode::serde::encode_to_vec(self, CONFIG).map_err(StateError::Encode)
    }

    /// Replaces the console's state with `state` (from [`Self::save_state`]
    /// on the same ROM). The ROM, BIOS, settings, `.sav` path, cheats and
    /// Wi-Fi link stay.
    ///
    /// # Errors
    /// If `state` is not a state of this core version; the console is
    /// unchanged then.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), StateError> {
        let (mut loaded, _): (Self, usize) =
            bincode::serde::decode_from_slice(state, CONFIG).map_err(StateError::Decode)?;
        core::mem::swap(&mut loaded.config, &mut self.config);
        core::mem::swap(&mut loaded.arm7_bios, &mut self.arm7_bios);
        core::mem::swap(&mut loaded.arm9_bios, &mut self.arm9_bios);
        core::mem::swap(&mut loaded.cheats, &mut self.cheats);
        core::mem::swap(&mut loaded.cart.rom, &mut self.cart.rom);
        core::mem::swap(&mut loaded.cart.save_database, &mut self.cart.save_database);
        loaded.cart.backup.set_path(self.cart.backup.path().map(ToOwned::to_owned));
        loaded.wifi.take_link_from(&mut self.wifi);
        // The replaced console flushes its save memory as it drops.
        *self = loaded;
        Ok(())
    }

    /// The loaded ROM's identity, stored beside a state by front ends.
    pub fn rom_fingerprint(&self) -> RomFingerprint {
        self.cart.rom_fingerprint()
    }
}

/// Serde for `[[u8; B]; A]` with `B` above serde's 32-element limit.
pub(crate) mod nested_array {
    use serde::{Deserialize as _, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S: Serializer, const A: usize, const B: usize>(
        rows: &[[u8; B]; A],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.collect_seq(rows.iter().map(<[u8; B]>::as_slice))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const A: usize, const B: usize>(
        d: D,
    ) -> Result<[[u8; B]; A], D::Error> {
        let rows = Vec::<Vec<u8>>::deserialize(d)?;
        let mut out = [[0; B]; A];
        if rows.len() != A {
            return Err(D::Error::invalid_length(rows.len(), &"one row per engine"));
        }
        for (dst, src) in out.iter_mut().zip(&rows) {
            if src.len() != B {
                return Err(D::Error::invalid_length(src.len(), &"a full register block"));
            }
            dst.copy_from_slice(src);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_and_keeps_host_data() {
        let mut emu = Emulator::new();
        emu.main_ram[0x1234] = 0x5A;
        emu.io_2d[1][3] = 7;
        emu.cart.rom = vec![1, 2, 3];
        let state = emu.save_state().unwrap();

        emu.main_ram[0x1234] = 0;
        emu.io_2d[1][3] = 0;
        emu.load_state(&state).unwrap();
        assert_eq!(emu.main_ram[0x1234], 0x5A);
        assert_eq!(emu.io_2d[1][3], 7);
        assert_eq!(emu.cart.rom, [1, 2, 3], "the ROM is not in the state");
        assert!(emu.load_state(&state[..10]).is_err());
    }
}
