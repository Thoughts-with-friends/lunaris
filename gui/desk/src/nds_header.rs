// https://problemkaputt.de/gbatek.htm#dscartridgeheader

/// Nintendo DS ROM Header (0x0000–0x01FF)
///
/// This structure represents the first 0x200 bytes of a .nds file.
/// It contains metadata for both ARM9 and ARM7 binaries, filesystem
/// offset information (NitroFS), secure area configuration, icon/title
/// location, and CRC values.
///
/// All fields use little-endian representation.
/// Reference: GBATEK / DS cartridge header documentation.
///
/// NOTE:
/// - Fixed size: 512 bytes (0x200)
/// - Strings are not null-terminated; user must trim
/// - Includes Nintendo logo data and CRCs
#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(C)]
pub struct NdsHeader {
    /// Game title (not null-terminated)
    pub game_title: [u8; 12], // 000h
    /// Game code (XXXX)
    pub game_code: [u8; 4], // 00Ch
    /// Maker code (publisher ID)
    pub maker_code: [u8; 2], // 010h

    /// Unit code (0 = NDS, DSi bits also seen in later titles)
    pub unit_code: u8, // 012h
    /// Encryption seed / DSi flags
    pub encryption_seed: u8, // 013h
    /// Cartridge capacity code (2^n MB)
    pub device_capacity: u8, // 014h

    /// Reserved internal fields
    pub reserved1: [u8; 7], // 015h–01Bh
    /// DSi region lock flags (00 old carts)
    pub reserved_dsi: u8, // 01Ch
    /// Region code / compatibility flags
    pub region: u8, // 01Dh
    /// ROM version
    pub rom_version: u8, // 01Eh
    /// Auto-start flag (0x40)
    pub autostart: u8, // 01Fh

    // --- ARM9 section ---
    /// ARM9 binary offset (ROM address)
    pub arm9_rom_offset: u32, // 020h
    /// ARM9 entrypoint address
    pub arm9_entry_addr: u32, // 024h
    /// ARM9 load address in RAM
    pub arm9_ram_addr: u32, // 028h
    /// ARM9 binary size
    pub arm9_size: u32, // 02Ch

    // --- ARM7 section ---
    pub arm7_rom_offset: u32, // 030h
    pub arm7_entry_addr: u32, // 034h
    pub arm7_ram_addr: u32,   // 038h
    pub arm7_size: u32,       // 03Ch

    // // --- NitroFS directory + file table ---
    pub fnt_offset: u32, // 040h (File Name Table)
    pub fnt_size: u32,   // 044h
    pub fat_offset: u32, // 048h (File Allocation Table)
    pub fat_size: u32,   // 04Ch

    // --- Overlay tables ---
    pub arm9_ovl_offset: u32, // 050h
    pub arm9_ovl_size: u32,   // 054h
    pub arm7_ovl_offset: u32, // 058h
    pub arm7_ovl_size: u32,   // 05Ch

    // --- Commands and secure area ---
    pub normal_cmd: u32, // 060h
    pub key1_cmd: u32,   // 064h

    /// Icon & title bitmap block offset
    pub icon_title_offset: u32, // 068h
    /// Secure area CRC
    pub secure_area_crc: u16, // 06Ch
    /// Secure area delay / key1 timing
    pub secure_area_delay: u16, // 06Eh

    pub arm9_hook_addr: u32, // 070h
    pub arm7_hook_addr: u32, // 074h

    /// Known values in commercial ROMs; used by secure boot
    pub secure_area_disable: [u8; 8], // 078h

    /// Total ROM size
    pub total_rom_size: u32, // 080h
    /// Header size (typically 0x200)
    pub header_size: u32, // 084h

    pub reserved2: [u8; 8], // 088h–08Fh

    /// NAND control end address (debug retail only)
    pub nand_end: u16, // 094h
    /// NAND RW start (debug retail only)
    pub nand_rw_start: u16, // 096h

    pub reserved3: [u8; 0x18], // 098h–0AFh
    pub reserved4: [u8; 0x10], // 0B0h–0BFh

    /// 156-byte Nintendo logo graphic data
    pub nintendo_logo: [u8; 0x9C], // 0C0h–15Bh
    /// Nintendo logo CRC16
    pub logo_crc: u16, // 15Ch
    /// Header CRC16 (first 0x15E bytes)
    pub header_crc: u16, // 15Eh

    // --- Debug and unused ---
    pub debug_rom_offset: u32, // 160h
    pub debug_size: u32,       // 164h
    pub debug_ram_addr: u32,   // 168h

    pub reserved5: u32,        // 16Ch
    pub reserved6: [u8; 0x90], // 170h–1FFh padding
}

use winnow::{
    Parser,
    binary::{self, Endianness},
    error::ContextError,
    token::take,
};

impl NdsHeader {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::error::ReadableError> {
        let mut input = bytes;
        let header = Self::parser().parse_next(&mut input).map_err(|err| {
            let hex = crate::hexdump::to_string(bytes);
            let hex_pos = crate::hexdump::to_hexdump_pos(input.len());
            crate::error::ReadableError::from_context(
                winnow::error::ErrMode::Backtrack(err),
                &hex,
                hex_pos,
            )
        })?;
        Ok(header)
    }

    /// Parse a Nintendo DS ROM header (0x200 bytes)
    pub fn parser<'a>() -> impl Parser<&'a [u8], Self, ContextError> {
        move |input: &mut &[u8]| {
            {
                let le = Endianness::Little;

                // --- Basic fields ---
                let game_title = take(12usize).try_map(TryFrom::try_from).parse_next(input)?;
                let game_code = take(4usize).try_map(TryFrom::try_from).parse_next(input)?;
                let maker_code = take(2usize).try_map(TryFrom::try_from).parse_next(input)?;

                let unit_code = binary::u8.parse_next(input)?;
                let encryption_seed = binary::u8.parse_next(input)?;
                let device_capacity = binary::u8.parse_next(input)?;
                let reserved1 = take(7usize).try_map(TryFrom::try_from).parse_next(input)?;
                let reserved_dsi = binary::u8.parse_next(input)?;
                let region = binary::u8.parse_next(input)?;
                let rom_version = binary::u8.parse_next(input)?;
                let autostart = binary::u8.parse_next(input)?;

                // --- ARM9 section ---
                let arm9_rom_offset = binary::u32(le).parse_next(input)?;
                let arm9_entry_addr = binary::u32(le).parse_next(input)?;
                let arm9_ram_addr = binary::u32(le).parse_next(input)?;
                let arm9_size = binary::u32(le).parse_next(input)?;

                // --- ARM7 section ---
                let arm7_rom_offset = binary::u32(le).parse_next(input)?;
                let arm7_entry_addr = binary::u32(le).parse_next(input)?;
                let arm7_ram_addr = binary::u32(le).parse_next(input)?;
                let arm7_size = binary::u32(le).parse_next(input)?;

                // --- NitroFS / FAT ---
                let fnt_offset = binary::u32(le).parse_next(input)?;
                let fnt_size = binary::u32(le).parse_next(input)?;
                let fat_offset = binary::u32(le).parse_next(input)?;
                let fat_size = binary::u32(le).parse_next(input)?;

                // --- Overlay tables ---
                let arm9_ovl_offset = binary::u32(le).parse_next(input)?;
                let arm9_ovl_size = binary::u32(le).parse_next(input)?;
                let arm7_ovl_offset = binary::u32(le).parse_next(input)?;
                let arm7_ovl_size = binary::u32(le).parse_next(input)?;

                // --- Commands & secure area ---
                let normal_cmd = binary::u32(le).parse_next(input)?;
                let key1_cmd = binary::u32(le).parse_next(input)?;
                let icon_title_offset = binary::u32(le).parse_next(input)?;
                let secure_area_crc = binary::u16(le).parse_next(input)?;
                let secure_area_delay = binary::u16(le).parse_next(input)?;
                let arm9_hook_addr = binary::u32(le).parse_next(input)?;
                let arm7_hook_addr = binary::u32(le).parse_next(input)?;
                let secure_area_disable =
                    take(8usize).try_map(TryFrom::try_from).parse_next(input)?;

                let total_rom_size = binary::u32(le).parse_next(input)?;
                let header_size = binary::u32(le).parse_next(input)?;

                let reserved2 = take(8usize).try_map(TryFrom::try_from).parse_next(input)?;
                let nand_end = binary::u16(le).parse_next(input)?;
                let nand_rw_start = binary::u16(le).parse_next(input)?;
                let reserved3 = take(0x18usize)
                    .try_map(TryFrom::try_from)
                    .parse_next(input)?;
                let reserved4 = take(0x10usize)
                    .try_map(TryFrom::try_from)
                    .parse_next(input)?;

                // --- Nintendo logo block ---
                let nintendo_logo = take(0x9Cusize)
                    .try_map(TryFrom::try_from)
                    .parse_next(input)?;
                let logo_crc = binary::u16(le).parse_next(input)?;
                let header_crc = binary::u16(le).parse_next(input)?;

                // --- Debug / unused ---
                let debug_rom_offset = binary::u32(le).parse_next(input)?;
                let debug_size = binary::u32(le).parse_next(input)?;
                let debug_ram_addr = binary::u32(le).parse_next(input)?;
                let reserved5 = binary::u32(le).parse_next(input)?;
                let reserved6 = take(0x90usize)
                    .try_map(TryFrom::try_from)
                    .parse_next(input)?;

                Ok(Self {
                    game_title,
                    game_code,
                    maker_code,
                    unit_code,
                    encryption_seed,
                    device_capacity,
                    reserved1,
                    reserved_dsi,
                    region,
                    rom_version,
                    autostart,

                    arm9_rom_offset,
                    arm9_entry_addr,
                    arm9_ram_addr,
                    arm9_size,
                    arm7_rom_offset,
                    arm7_entry_addr,
                    arm7_ram_addr,
                    arm7_size,

                    fnt_offset,
                    fnt_size,
                    fat_offset,
                    fat_size,

                    arm9_ovl_offset,
                    arm9_ovl_size,
                    arm7_ovl_offset,
                    arm7_ovl_size,

                    normal_cmd,
                    key1_cmd,
                    icon_title_offset,
                    secure_area_crc,
                    secure_area_delay,
                    arm9_hook_addr,
                    arm7_hook_addr,
                    secure_area_disable,

                    total_rom_size,
                    header_size,
                    reserved2,
                    nand_end,
                    nand_rw_start,
                    reserved3,
                    reserved4,

                    nintendo_logo,
                    logo_crc,
                    header_crc,
                    debug_rom_offset,
                    debug_size,
                    debug_ram_addr,
                    reserved5,
                    reserved6,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_parse_header() {
        let input = std::fs::read(r"").unwrap();
        let res = NdsHeader::from_bytes(&input).unwrap_or_else(|e| {
            std::fs::write("./error.txt", format!("{e}")).unwrap();
            panic!("failed to parse");
        });
        std::fs::write("./nds_header_debug.txt", format!("{res:#?}"))
            .expect("failed to write debug file");
    }
}
