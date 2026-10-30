//! DS BIOS and firmware: built-in replacement images and the high-level
//! BIOS (SWI) implementation.
pub mod firmware;
pub mod image;
pub mod swi;

#[cfg(test)]
mod tests {
    use super::{
        firmware::FIRMWARE_DS,
        image::{arm7::BIOS_ARM7_BIN, arm9::BIOS_ARM9_BIN},
    };

    fn create_dir_all() -> std::io::Result<&'static std::path::Path> {
        let dir_name: &std::path::Path = std::path::Path::new("bios");
        std::fs::create_dir_all(dir_name)?;
        Ok(dir_name)
    }

    #[test]
    fn test_create_arm() -> std::io::Result<()> {
        let dir_name = create_dir_all()?;

        // ARM7 BIOS
        std::fs::write(dir_name.join("bios_arm7.bin"), BIOS_ARM7_BIN)?;
        println!("Saved bios_arm7.bin ({} bytes)", BIOS_ARM7_BIN.len());

        // ARM9 BIOS
        std::fs::write(dir_name.join("bios_arm9.bin"), BIOS_ARM9_BIN)?;
        println!("Saved bios_arm9.bin ({} bytes)", BIOS_ARM9_BIN.len());

        Ok(())
    }

    #[test]
    fn test_create_firmware() -> std::io::Result<()> {
        let dir_name = create_dir_all()?;

        // Firmware
        std::fs::write(dir_name.join("firmware.bin"), FIRMWARE_DS)?;
        println!("Saved firmware.bin ({} bytes)", FIRMWARE_DS.len());

        // Basic sanity check for first few bytes
        assert_eq!(FIRMWARE_DS[0x08], b'M');
        // DS Firmware Header: https://problemkaputt.de/gbatek.htm#dsfirmwareheader
        assert_eq!(FIRMWARE_DS[0x09], b'A'); // firmware header 08h-0Bh: "MAC" + firmware ID
        assert_eq!(FIRMWARE_DS[0x0A], b'C'); // firmware ID
        assert_eq!(FIRMWARE_DS[0x0B], 0x68); // firmware ID / version byte

        println!("Basic sanity check passed.");
        Ok(())
    }
}
