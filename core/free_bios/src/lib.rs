pub mod arm7;
pub mod arm9;
pub mod firmware;

#[cfg(test)]
mod tests {
    use super::arm7::BIOS_ARM7_BIN;
    use super::arm9::BIOS_ARM9_BIN;
    use super::firmware::{DSType, default_firmware};

    fn create_dir_all() -> std::io::Result<&'static std::path::Path> {
        let dir_name: &std::path::Path = std::path::Path::new("bios");
        std::fs::create_dir_all(dir_name)?;
        Ok(dir_name)
    }

    #[test]
    fn test_create_arm() -> std::io::Result<()> {
        let dir_name = create_dir_all()?;

        // ARM7 BIOS
        std::fs::write(dir_name.join("bios_arm7.bin"), &BIOS_ARM7_BIN)?;
        println!("Saved bios_arm7.bin ({} bytes)", BIOS_ARM7_BIN.len());

        // ARM9 BIOS
        std::fs::write(dir_name.join("bios_arm9.bin"), &BIOS_ARM9_BIN)?;
        println!("Saved bios_arm9.bin ({} bytes)", BIOS_ARM9_BIN.len());

        Ok(())
    }

    #[test]
    fn test_create_firmware() -> std::io::Result<()> {
        let dir_name = create_dir_all()?;

        // Firmware
        const FW: [u8; 0x40000] = default_firmware::<0x40000>(DSType::Ds);
        std::fs::write(dir_name.join("firmware.bin"), &FW)?;
        println!("Saved firmware.bin ({} bytes)", FW.len());

        // Basic sanity check for first few bytes
        assert_eq!(FW[0x08], b'M');
        assert_eq!(FW[0x09], b'A');
        assert_eq!(FW[0x0A], b'C');
        assert_eq!(FW[0x0B], 0x68);

        println!("Basic sanity check passed.");
        Ok(())
    }
}
