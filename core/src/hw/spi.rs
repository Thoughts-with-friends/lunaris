//! ARM7 Serial Peripheral Interface bus (SPICNT / SPIDATA, 40001C0h).
//!
//! The SPI bus connects the ARM7 to three devices selected by SPICNT bits
//! 8-9: power-management chip, firmware serial flash, and the touch-screen
//! controller (TSC).
//!
//! GBATEK references:
//! - SPI bus / SPICNT / SPIDATA: <https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi>
//! - Firmware flash commands: <https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory>
//! - Firmware user settings (touch calibration): <https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings>

mod tsc;

use std::path::PathBuf;

use tsc::TSC;

use super::{GPU, HW, Scheduler, mem::IORegister};
use crate::hw::cartridge::{Backup, BackupProtocolState, Flash, SaveMem};

#[derive(emu_utils::Savestate)]
#[load(in_place_only)]
pub struct SPI {
    cnt: CNT,
    /// Not serialized directly; re-opened from the firmware file at
    /// [`SPI::new`] time. Its in-flight SPI protocol state is
    /// captured/restored via `firmware_protocol` below, mirroring
    /// `Cartridge::backup_protocol`. See
    /// `docs/design/savestate-and-video-design.md` §2.3.
    #[savestate(skip)]
    firmware: Flash,
    #[store(with = "save.store(&mut firmware.protocol_snapshot())?")]
    #[load(with_in_place = "firmware.restore_protocol_state(save.load()?)")]
    firmware_protocol: BackupProtocolState,
    tsc: TSC,
}

impl SPI {
    pub fn new(firmware_path: PathBuf) -> Self {
        SPI {
            cnt: CNT::new(),
            firmware: Flash::new_firmware(SPI::init_firmware(firmware_path)),
            firmware_protocol: BackupProtocolState::None,
            tsc: TSC::new(),
        }
    }

    pub fn read_cnt(&self, byte: usize) -> u8 {
        if self.cnt.enable { self.cnt.read(byte) } else { 0 }
    }
    pub fn read_data(&self) -> u8 {
        match self.cnt.device {
            Device::Firmware => self.firmware.read(),
            Device::Touchscreen => self.tsc.read(),
            _ => 0,
        }
    }

    pub fn write_cnt(&mut self, scheduler: &mut Scheduler, byte: usize, value: u8) {
        let prev_enable = self.cnt.enable;
        let prev_device = self.cnt.device;
        self.cnt.write(scheduler, byte, value);
        if prev_enable && !self.cnt.enable {
            // Disabling requires device to be reset for libnds to work
            match prev_device {
                Device::Firmware => self.firmware.deselect(),
                Device::Touchscreen => self.tsc.deselect(),
                _ => (),
            }
        }
    }

    pub fn write_data(&mut self, value: u8) {
        if !self.cnt.enable {
            return;
        }
        match self.cnt.device {
            Device::Firmware => self.firmware.write(self.cnt.hold, value),
            Device::Touchscreen => self.tsc.write(value),
            _ => (),
        }
    }

    pub fn press_screen(&mut self, x: usize, y: usize) {
        self.tsc.press_screen(x, y)
    }

    /// Returns the firmware's Wi-Fi calibration block (offset `02Ch`
    /// onward), for [`crate::hw::HW::load_wifi_firmware_config`]. See
    /// `docs/design/design_lan.md` §7.1.
    pub fn wifi_config_bytes(&self) -> Option<&[u8]> {
        self.firmware.save_bytes().and_then(|b| b.get(0x2C..))
    }

    /// The firmware header's `ConsoleType` byte (offset `01Dh`).
    ///
    /// The Wi-Fi hardware identifies itself differently per console revision,
    /// and the DS driver reads that id during init, so it has to come from the
    /// image rather than being assumed. See [`crate::hw::net::Wifi::reset`].
    pub fn firmware_console_type(&self) -> Option<u8> {
        self.firmware.save_bytes().and_then(|b| b.get(0x1D).copied())
    }
    pub fn release_screen(&mut self) {
        self.tsc.release_screen()
    }
    /// Loads the firmware image into memory and patches the user-settings
    /// area (offset 3FE00h) with touch-screen calibration matching the
    /// emulated screen corners, then fixes up the settings CRC16. The patch
    /// is flushed to disk immediately (see [`SaveMem::flush`]); no file
    /// mapping is held afterwards, so the firmware file is never locked for
    /// the emulator's lifetime the way the previous `mmap`-backed
    /// implementation locked it. See
    /// `docs/design/sav-backup-redesign.md` §4.1.
    ///
    /// GBATEK "DS Firmware User Settings" (calibration bytes 58h-63h,
    /// CRC at 72h): <https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings>
    pub fn init_firmware(firmware_path: PathBuf) -> SaveMem {
        let mut mem = SaveMem::open_existing(firmware_path).unwrap();
        let firmware = mem.bytes_mut();
        let user_settings_addr = 0x3FE00;

        // Set Touch Screen Calibration
        let max_x = GPU::WIDTH - 1;
        let max_y = GPU::HEIGHT - 1;
        // Top Left Corner
        HW::write_mem(firmware, user_settings_addr + 0x58, 0u16);
        HW::write_mem(firmware, user_settings_addr + 0x5A, 0u16);
        firmware[user_settings_addr as usize + 0x5C] = 0;
        firmware[user_settings_addr as usize + 0x5D] = 0;
        // Bottom Right Corner
        HW::write_mem(firmware, user_settings_addr + 0x5E, (max_x as u16) << 4);
        HW::write_mem(firmware, user_settings_addr + 0x60, (max_y as u16) << 4);
        firmware[user_settings_addr as usize + 0x62] = max_x as u8;
        firmware[user_settings_addr as usize + 0x63] = max_y as u8;
        let crc16 = {
            let mut crc = 0xFFFF;
            let vals = [0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001];
            for byte in
                firmware[user_settings_addr as usize..user_settings_addr as usize + 0x70].iter()
            {
                crc ^= *byte as u32;
                for (i, val) in vals.iter().enumerate() {
                    let new_crc = crc >> 1;
                    crc = if crc & 0x1 != 0 {
                        // Carry Occurred
                        new_crc ^ (val << (7 - i))
                    } else {
                        new_crc
                    };
                }
            }
            crc as u16
        };
        HW::write_mem(firmware, user_settings_addr + 0x72, crc16);
        mem.flush();
        mem
    }
}

/// 40001C0h - NDS7 - SPICNT: baud rate, device select, transfer size,
/// chip-select hold, IRQ, and bus enable.
///
/// GBATEK: <https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi>
#[derive(emu_utils::Savestate)]
pub struct CNT {
    baudrate: u8,
    busy: bool,
    device: Device,
    transfer16: bool,
    hold: bool,
    irq: bool,
    enable: bool,
}

impl CNT {
    pub fn new() -> Self {
        CNT {
            baudrate: 0,
            busy: false,
            device: Device::Powerman,
            transfer16: false,
            hold: false,
            irq: false,
            enable: false,
        }
    }
}

impl IORegister for CNT {
    fn read(&self, byte: usize) -> u8 {
        match byte {
            0 => (self.busy as u8) << 7 | self.baudrate,
            1 => {
                (self.enable as u8) << 7
                    | (self.irq as u8) << 6
                    | (self.hold as u8) << 3
                    | (self.transfer16 as u8) << 2
                    | (self.device as u8)
            }
            _ => unreachable!(),
        }
    }

    fn write(&mut self, _scheduler: &mut Scheduler, byte: usize, value: u8) {
        match byte {
            0 => {
                // TODO: Set busy flag properly
                self.baudrate = value & 0x3;
            }
            1 => {
                self.enable = value >> 7 & 0x1 != 0;
                self.irq = value >> 6 & 0x1 != 0;
                assert!(!self.irq);
                self.hold = value >> 3 & 0x1 != 0;
                self.transfer16 = value >> 2 & 0x1 != 0;
                assert!(!self.transfer16);
                self.device = Device::from_bits(value & 0x3);
            }
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
enum Device {
    Powerman = 0,
    Firmware = 1,
    Touchscreen = 2,
}

impl Device {
    pub fn from_bits(value: u8) -> Self {
        match value {
            0 => Self::Powerman,
            1 => Self::Firmware,
            2 => Self::Touchscreen,
            3 => panic!("Reserved SPI Device!"),
            _ => unreachable!(),
        }
    }
}
