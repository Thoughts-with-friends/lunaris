// SPDX-FileCopyrightText: (C) 2007 kelpsyberry
// SPDX-License-Identifier: GPL-3.0-or-later
// https://github.com/kelpsyberry/dust/blob/main/core/src/spi/firmware.rs#L8

/// DSType enum
/// DS, Lite, DSi, iQue DS, iQue DS Lite
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
pub enum DSType {
    Ds,
    Lite,
    Dsi,
    /// iQue: Chinese Nintendo DS, uses region-locked cartridges and localized OS
    Ique,
    IqueLite,
}

impl DSType {
    /// Firmware header byte 0x1D (console type) of the model.
    const fn model_spec(&self) -> u8 {
        match self {
            // DS Firmware Header: https://problemkaputt.de/gbatek.htm#dsfirmwareheader
            Self::Ds => 0xFF,       // console type FFh: Nintendo DS
            Self::Lite => 0x20,     // console type 20h: Nintendo DS-Lite
            Self::Ique => 0x57,     // console type 57h: GBATEK lists Nintendo DSi (also iQue DSi)
            Self::IqueLite => 0x43, // console type 43h: GBATEK lists iQue DS
            Self::Dsi => 0x63,      // console type 63h: GBATEK lists iQue DS-Lite
        }
    }
}

/// DS firmware for standard DS/Lite (256KB)
///
/// # Example
/// ```no_run
/// use ds_bios::firmware::FIRMWARE_DS;
/// std::fs::write("firmware.bin", FIRMWARE_DS).unwrap();
/// ```
pub static FIRMWARE_DS: [u8; 0x40000] = default_firmware::<0x40000>(DSType::Ds);
/// DS firmware for DSi (128KB)
pub static FIRMWARE_DSI: [u8; 0x20000] = default_firmware::<0x20000>(DSType::Dsi);
/// DS firmware for iQue DS (512KB)
pub static FIRMWARE_DS_IQUE: [u8; 0x80000] = default_firmware::<0x80000>(DSType::Ique);

/// Wi-Fi MAC address of the built-in firmware (melonDS' default; 00:09:BF
/// is Nintendo's OUI).
///
/// It must not be all zeros: the SDK copies it to 0x027FFCF4 at boot
/// (`OS_GetMacAddress`), and the anti-piracy code of later games (e.g.
/// Pokemon + Nobunaga's Ambition) treats a zero MAC as an emulator and calls
/// `OS_Terminate`.
const MAC_ADDRESS: [u8; 6] = [0x00, 0x09, 0xBF, 0x11, 0x22, 0x33];

/// User settings 0x58..0x63: ADC (x1, y1) = (0x200, 0x200) at pixel
/// (32, 32) and ADC (x2, y2) = (0xE00, 0xA00) at pixel (224, 160).
const TOUCH_CALIBRATION: [u8; 12] = [
    // DS Firmware User Settings: https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings
    0x00, 0x02, 0x00, 0x02, 32, 32, 0x00, 0x0E, 0x00, 0x0A, 224,
    160, // ADC (200h,200h)->scr (32,32), ADC (E00h,A00h)->scr (224,160)
];

/// Wi-Fi calibration 0x44..0x63: initial values of the Wi-Fi I/O ports
/// (melonDS `FirmwareHeader`).
const WIFI_INIT_PORTS: [u16; 16] = [
    // DS Firmware Wifi Calibration Data: https://problemkaputt.de/gbatek.htm#dsfirmwarewificalibrationdata
    0x0002, 0x0017, 0x0026, 0x1818, 0x0048, 0x4840, 0x0058, 0x0042, 0x0146, 0x8064, 0xE6E6,
    0x2443, // initial values for W_CONFIG ports (firmware 044h-063h)
    0x000E, 0x0001, 0x0001, 0x0402, // initial values for W_CONFIG ports (continued)
];

/// Wi-Fi calibration 0x64..0xCC: initial baseband registers 0x00..0x68.
const WIFI_BB_INIT: [u8; 0x69] = [
    // DS Firmware Wifi Calibration Data: https://problemkaputt.de/gbatek.htm#dsfirmwarewificalibrationdata
    0x03, 0x17, 0x40, 0x00, 0x1B, 0x6C, 0x48, 0x80, 0x38, 0x00, 0x35, 0x07, 0x00, 0x00, 0x00,
    0x00, // initial 8-bit values for BB[00h..68h] (firmware 064h)
    0x00, 0x00, 0x00, 0x00, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0xBB, 0x01, 0x24,
    0x7F, // BB init (continued)
    0x5A, 0x01, 0x3F, 0x01, 0x3F, 0x36, 0x1D, 0x00, 0x78, 0x35, 0x55, 0x12, 0x34, 0x1C, 0x00,
    0x01, // BB init (continued)
    0x0E, 0x38, 0x03, 0x70, 0xC5, 0x2A, 0x0A, 0x08, 0x04, 0x01, 0x00, 0x00, 0x00, 0xFF, 0xFF,
    0xFE, // BB init (continued)
    0xFE, 0xFE, 0xFE, 0xFC, 0xFC, 0xFA, 0xFA, 0xFA, 0xFA, 0xFA, 0xF8, 0xF8, 0xF6, 0x00, 0x12,
    0x14, // BB init (continued)
    0x12, 0x41, 0x23, 0x03, 0x04, 0x70, 0x35, 0x0E, 0x2C, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // BB init (continued)
    0x00, 0x00, 0x00, 0x0E, 0x00, 0x00, 0x12, 0x28, 0x1C, // BB init (continued)
];

/// Wi-Fi calibration 0xCE..0xF6: initial RF registers of an RF chip type 3.
const WIFI_RF_INIT: [u8; 0x29] = [
    // DS Firmware Wifi Calibration Data: https://problemkaputt.de/gbatek.htm#dsfirmwarewificalibrationdata
    0x31, 0x4C, 0x4F, 0x21, 0x00, 0x10, 0xB0, 0x08, 0xFA, 0x15, 0x26, 0xE6, 0xC1, 0x01, 0x0E,
    0x50, // initial 8-bit values for RF[00h..28h] (firmware 0CEh)
    0x05, 0x00, 0x6D, 0x12, 0x00, 0x00, 0x01, 0xFF, 0x0E, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00,
    0x06, // RF init (continued)
    0x06, 0x00, 0x00, 0x00, 0x18, 0x00, 0x02, 0x00, 0x00, // RF init (continued)
];

/// Wi-Fi calibration 0xF8..0x133: per-channel BB/RF values (type 3).
const WIFI_CHANNEL_DATA: [u8; 0x3C] = [
    // DS Firmware Wifi Calibration Data: https://problemkaputt.de/gbatek.htm#dsfirmwarewificalibrationdata
    0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x16,
    0x26, // per-channel BB/RF values (firmware 0F7h..)
    0x1C, 0x1C, 0x1C, 0x1D, 0x1D, 0x1D, 0x1E, 0x1E, 0x1E, 0x1E, 0x1F, 0x1E, 0x1F, 0x18, 0x01,
    0x4B, // channel data (continued)
    0x4B, 0x4B, 0x4B, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4D, 0x4D, 0x4D, 0x02, 0x6C,
    0x71, // channel data (continued)
    0x76, 0x5B, 0x40, 0x45, 0x4A, 0x2F, 0x34, 0x39, 0x3E, 0x03, 0x08,
    0x14, // channel data (continued)
];

/// Length of the Wi-Fi config block at 0x2C (GBATEK "DS Firmware Header").
const WIFI_CONFIG_LEN: usize = 0x138;

// CRC16 calculation using while loop, const compatible
/// CRC16 of a 0x70-byte user settings block.
const fn crc16(user: [u8; 0x70]) -> u16 {
    // DS Firmware User Settings: https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings
    crc16_range(&user, 0, 0x70, 0xFFFF) // CRC16 over 70h bytes of user settings, initial FFFFh
}

/// CRC16 (poly 0xA001) of `data[start..start + len]`, const compatible.
const fn crc16_range(data: &[u8], start: usize, len: usize, init: u16) -> u16 {
    let mut crc = init;
    let mut i = start;
    while i < start + len {
        crc ^= data[i] as u16;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                // BIOS Misc Functions: https://problemkaputt.de/gbatek.htm#biosmiscfunctions
                crc = (crc >> 1) ^ 0xA001; // CRC16 polynomial (reflected 8005h)
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        i += 1;
    }
    crc
}

/// Const-compatible firmware creation
const fn default_firmware<const N: usize>(model: DSType) -> [u8; N] {
    let mut firmware = [0_u8; N];

    // 0x04..0x07
    firmware[0x04] = 0x00;
    // DS Firmware Header: https://problemkaputt.de/gbatek.htm#dsfirmwareheader
    firmware[0x05] = 0xDB; // part3/4 CRC16 (high byte) / part1/2 CRC16 (006h-007h)
    firmware[0x06] = 0x1F; // part1/2 CRC16 arm9/7 boot code
    firmware[0x07] = 0x0F; // part1/2 CRC16 arm9/7 boot code

    // 0x08..0x0B "MACh"
    // TODO: optional customization
    firmware[0x08] = b'M';
    firmware[0x09] = b'A'; // firmware identifier "MAC"
    firmware[0x0A] = b'C'; // firmware identifier "MAC"
    firmware[0x0B] = 0x68; // firmware identifier version byte

    // 0x14..0x15
    let val14: u16 = ((N >> 17) << 12) as u16;
    firmware[0x14] = (val14 & 0xFF) as u8; // 014h: shift amounts / firmware size field
    firmware[0x15] = (val14 >> 8) as u8; // 014h high byte

    // 0x18..0x1C
    firmware[0x18] = 0x00;
    firmware[0x19] = 0x00; // 018h: firmware build timestamp (BCD minute, hour, day, month, year)
    firmware[0x1A] = 0x01; // timestamp: BCD day 01
    firmware[0x1B] = 0x01; // timestamp: BCD month 01
    firmware[0x1C] = 0x06; // timestamp: BCD year 06

    // 0x1D model-specific
    firmware[0x1D] = model.model_spec();

    // 0x1E..0x1F
    firmware[0x1E] = 0xFF;
    firmware[0x1F] = 0xFF; // 01Eh-01Fh: unused (FFh-filled)

    // 0x2C..0x163: Wi-Fi config (length, MAC, enabled channels), CRC at 0x2A
    firmware[0x2C] = (WIFI_CONFIG_LEN & 0xFF) as u8;
    // DS Firmware Wifi Calibration Data: https://problemkaputt.de/gbatek.htm#dsfirmwarewificalibrationdata
    firmware[0x2D] = (WIFI_CONFIG_LEN >> 8) as u8; // 02Ch: config_length (high byte)
    let mut i = 0;
    while i < 6 {
        firmware[0x36 + i] = MAC_ADDRESS[i]; // 036h: 48-bit MAC address
        i += 1;
    }
    firmware[0x3C] = 0xFE; // 03Ch: enabled channels 1-13 (low byte)
    firmware[0x3D] = 0x3F; // 03Ch: enabled channels (high byte)

    // Wi-Fi calibration (melonDS' generated firmware). Without it the ARM7
    // wireless library cannot program the baseband/RF chips, WM_Enable fails
    // and games treat that as a communication error (Pokemon HeartGold then
    // stops reading the touch screen on its title menu).
    firmware[0x2F] = 6; // Wi-Fi version W006
    let mut i = 0;
    while i < 5 {
        firmware[0x30 + i] = 0xFF; // 030h: unused (FFh-filled on DS-Lite/DSi)
        i += 1;
    }
    firmware[0x3E] = 0xFF; // 03Eh: whatever flags (usually FFFFh)
    firmware[0x3F] = 0xFF; // 03Eh: whatever flags (high byte)
    firmware[0x40] = 3; // RF chip type 3
    firmware[0x41] = 0x94; // RF bits per entry
    firmware[0x42] = 0x29; // RF entries
    firmware[0x43] = 0x02; // 043h: unknown
    let mut i = 0;
    while i < WIFI_INIT_PORTS.len() {
        firmware[0x44 + i * 2] = (WIFI_INIT_PORTS[i] & 0xFF) as u8; // 044h: initial W_CONFIG port values
        firmware[0x45 + i * 2] = (WIFI_INIT_PORTS[i] >> 8) as u8; // 044h: initial W_CONFIG port values (high byte)
        i += 1;
    }
    let mut i = 0;
    while i < WIFI_BB_INIT.len() {
        firmware[0x64 + i] = WIFI_BB_INIT[i]; // 064h: initial BB values
        i += 1;
    }
    let mut i = 0;
    while i < WIFI_RF_INIT.len() {
        firmware[0xCE + i] = WIFI_RF_INIT[i]; // 0CEh: initial RF values
        i += 1;
    }
    firmware[0xF7] = 0x02; // BB indices per channel
    let mut i = 0;
    while i < WIFI_CHANNEL_DATA.len() {
        firmware[0xF8 + i] = WIFI_CHANNEL_DATA[i]; // 0F8h: per-channel data
        i += 1;
    }
    let mut i = 0x134; // unused area 134h-161h (FFh-filled)
    while i < 0x162 {
        // end of the Wifi config area
        firmware[i] = 0xFF; // FFh-filled
        i += 1;
    }
    let wifi_crc = crc16_range(&firmware, 0x2C, WIFI_CONFIG_LEN, 0x0000); // 02Ah: CRC16 (initial 0) over 2Ch..2Ch+config_length-1
    firmware[0x2A] = (wifi_crc & 0xFF) as u8; // 02Ah: Wifi config CRC16 (low byte)
    firmware[0x2B] = (wifi_crc >> 8) as u8; // 02Ah: Wifi config CRC16 (high byte)

    // 0x20..0x28 values
    let values: [u16; 5] = [((N - 0x200) >> 3) as u16, 0x0B51, 0x0DB3, 0x4F5D, 0xFFFF];
    let mut idx = 0;
    while idx < 5 {
        let val = values[idx];
        // DS Firmware Header: https://problemkaputt.de/gbatek.htm#dsfirmwareheader
        firmware[0x20 + idx * 2] = (val & 0xFF) as u8; // 020h-029h: user settings offset/8, unknown 022h/024h, part5 CRC16 (026h), unused (028h); low byte
        firmware[0x21 + idx * 2] = (val >> 8) as u8; // high byte
        idx += 1;
    }

    // User settings for 2 users
    let mut u = 0;
    while u < 2 {
        // DS Firmware User Settings: https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings
        let start = N - 0x200 + u * 0x100; // two 100h-byte user settings copies in the last 200h bytes
        let mut i = 0;
        while i < 0x100 {
            // 100h bytes per copy
            firmware[start + i] = 0;
            i += 1;
        }

        firmware[start] = 5;
        firmware[start + 0x02] = if u == 0 { 1 } else { 0 }; // 002h: favorite color
        firmware[start + 0x03] = 1; // 003h: birthday month
        firmware[start + 0x04] = 1; // 004h: birthday day

        // Name "Luna"
        firmware[start + 0x06] = b'L';
        firmware[start + 0x07] = 0; // 006h: nickname (UTF-16)
        firmware[start + 0x08] = b'u'; // nickname character
        firmware[start + 0x09] = 0; // nickname character
        firmware[start + 0x0A] = b'n'; // nickname character
        firmware[start + 0x0B] = 0; // nickname character
        firmware[start + 0x0C] = b'a'; // nickname character
        firmware[start + 0x0D] = 0; // nickname character

        // 0x58..0x63: touch screen calibration (GBATEK "DS Firmware User
        // Settings"): two points as 12-bit ADC values plus the screen pixel
        // they correspond to. The emulated TSC reports `pixel << 4`, so the
        // points match that mapping. All-zero calibration (as before) gives
        // the SDK's TP_CalcCalibrateParam nothing to work with.
        let mut c = 0;
        while c < TOUCH_CALIBRATION.len() {
            firmware[start + 0x58 + c] = TOUCH_CALIBRATION[c]; // 058h: touch-screen calibration points
            c += 1;
        }

        // CRC16
        let mut crc: [u8; 112] = [0; 0x70];
        let mut i = 0;
        while i < 0x70 {
            // CRC covers entries 00h..6Fh
            crc[i] = firmware[start + i];
            i += 1;
        }
        let crc: u16 = crc16(crc);

        firmware[start + 0x72] = (crc & 0xFF) as u8; // 072h: CRC16 (low byte)
        firmware[start + 0x73] = (crc >> 8) as u8; // 072h: CRC16 (high byte)

        u += 1;
    }

    firmware
}
