//! A compact identity of the loaded ROM, compared before a savestate is
//! applied (the ROM itself is not part of the state).

/// Game code (header 0x0C), header checksum (0x15E), secure area checksum
/// (0x6C) and ROM length. Cheap to compute even for 100+ MB ROMs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RomFingerprint {
    pub game_code: u32,
    pub header_checksum: u16,
    pub secure_area_checksum: u16,
    pub rom_len: u64,
}

impl RomFingerprint {
    /// Length of [`Self::to_bytes`].
    pub const ENCODED_LEN: usize = 16;

    /// The fingerprint of the ROM image `rom` (a short image reads as 0s).
    pub fn of_rom(rom: &[u8]) -> Self {
        let u16_at =
            |at: usize| rom.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let u32_at = |at: usize| u32::from(u16_at(at)) | (u32::from(u16_at(at + 2)) << 16);
        Self {
            game_code: u32_at(0x0C),
            header_checksum: u16_at(0x15E),
            secure_area_checksum: u16_at(0x6C),
            rom_len: rom.len() as u64,
        }
    }

    /// Little-endian encoding: game code, header checksum, secure area
    /// checksum, ROM length.
    pub fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut out = [0_u8; Self::ENCODED_LEN];
        out[0..4].copy_from_slice(&self.game_code.to_le_bytes());
        out[4..6].copy_from_slice(&self.header_checksum.to_le_bytes());
        out[6..8].copy_from_slice(&self.secure_area_checksum.to_le_bytes());
        out[8..16].copy_from_slice(&self.rom_len.to_le_bytes());
        out
    }

    pub const fn from_bytes(b: &[u8; Self::ENCODED_LEN]) -> Self {
        Self {
            game_code: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            header_checksum: u16::from_le_bytes([b[4], b[5]]),
            secure_area_checksum: u16::from_le_bytes([b[6], b[7]]),
            rom_len: u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_reads_the_header() {
        let mut rom = vec![0_u8; 0x200];
        rom[0x0C..0x10].copy_from_slice(b"ABCD");
        rom[0x15E] = 0x34;
        rom[0x15F] = 0x12;
        let fp = RomFingerprint::of_rom(&rom);
        assert_eq!(fp.game_code, u32::from_le_bytes(*b"ABCD"));
        assert_eq!(fp.header_checksum, 0x1234);
        assert_eq!(fp.rom_len, 0x200);
        assert_eq!(RomFingerprint::from_bytes(&fp.to_bytes()), fp);
    }
}
