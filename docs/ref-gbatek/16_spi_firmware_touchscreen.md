# 16. SPI: Firmware, Touchscreen, Power

Three devices share one two-register serial bus on the ARM7. This is where the
console's identity lives — its calibration, its user settings, its Wi-Fi
calibration — and where the stylus comes from.

GBATEK references:
[SPI bus](https://problemkaputt.de/gbatek.htm#dsserialperipheralinterfacebusspi) ·
[Firmware serial flash](https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory) ·
[Firmware user settings](https://problemkaputt.de/gbatek.htm#dsfirmwareusersettings) ·
[Touch screen controller](https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc) ·
[Power control](https://problemkaputt.de/gbatek.htm#dspowercontrol)

---

## 16.1 One bus, three devices

```text
                        ARM7
                          │
             SPICNT 40001C0h  ── device select (bits 8-9), hold, IRQ, enable
             SPIDATA 40001C2h ── one byte in / one byte out per access
                          │
        ┌─────────────────┼─────────────────┐
        ▼                 ▼                 ▼
   ┌──────────┐   ┌───────────────┐   ┌──────────────┐
   │ Powerman │   │ Firmware      │   │ TSC2046      │
   │ device 0 │   │ device 1      │   │ device 2     │
   │          │   │ 256 KB serial │   │ touchscreen  │
   │ backlight│   │ flash:        │   │ + microphone │
   │ power    │   │  Wi-Fi cal    │   │              │
   │ (stub)   │   │  user settings│   │              │
   └──────────┘   └───────────────┘   └──────────────┘
```

[spi.rs:21-36](core/src/hw/spi.rs#L21-L36):

```rust
pub struct SPI {
    cnt: CNT,
    /// Not serialized directly; re-opened from the firmware file at
    /// [`SPI::new`] time. Its in-flight SPI protocol state is
    /// captured/restored via `firmware_protocol` below, mirroring
    /// `Cartridge::backup_protocol`.
    #[savestate(skip)]
    firmware: Flash,
    #[store(with = "save.store(&mut firmware.protocol_snapshot())?")]
    #[load(with_in_place = "firmware.restore_protocol_state(save.load()?)")]
    firmware_protocol: BackupProtocolState,
    tsc: TSC,
}
```

The firmware reuses the **same `Flash` type** as a cartridge save chip
(Chapter 15) — it is the same kind of serial flash, so the same state machine
and the same savestate treatment apply.

Routing is a two-line match ([spi.rs:51-82](core/src/hw/spi.rs#L51-L82)):

```rust
    pub fn read_data(&self) -> u8 {
        match self.cnt.device {
            Device::Firmware => self.firmware.read(),
            Device::Touchscreen => self.tsc.read(),
            _ => 0,
        }
    }
```

### Deselect matters

```rust
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
```

([spi.rs:59-71](core/src/hw/spi.rs#L59-L71))

Disabling the whole bus, not just clearing hold, must reset the selected
device's state machine. The comment names the symptom: libnds gets out of sync
otherwise.

---

## 16.2 The firmware image

```text
   firmware.bin (256 KB)
   00000h ┌────────────────────────────────────────────┐
          │ header                                      │
          │  01Dh console type ──────► Wi-Fi identifies │
          │  02Ch Wi-Fi RF calibration block ─────────► │  load_wifi_firmware_config
          ├────────────────────────────────────────────┤
          │ Wi-Fi config, boot code, menu graphics,     │
          │ fonts, ...                                 │
   3FE00h ├────────────────────────────────────────────┤
          │ USER SETTINGS (2 copies, 0x100 bytes each) │
          │  ...                                       │
          │  058h touch calibration ADC x1             │
          │  05Ah touch calibration ADC y1             │
          │  05Ch/05Dh screen x1 / y1                  │
          │  05Eh touch calibration ADC x2             │
          │  060h touch calibration ADC y2             │
          │  062h/063h screen x2 / y2                  │
          │  ...                                       │
          │  072h CRC16 over 000h..06Fh                │
   40000h └────────────────────────────────────────────┘
```

Two small pieces are read by the emulator itself
([spi.rs:88-102](core/src/hw/spi.rs#L88-L102)):

```rust
    /// Returns the firmware's Wi-Fi calibration block (offset `02Ch`
    /// onward), for [`crate::hw::HW::load_wifi_firmware_config`].
    pub fn wifi_config_bytes(&self) -> Option<&[u8]> {
        self.firmware.save_bytes().and_then(|b| b.get(0x2C..))
    }

    /// The firmware header's `ConsoleType` byte (offset `01Dh`).
    ///
    /// The Wi-Fi hardware identifies itself differently per console revision,
    /// and the DS driver reads that id during init, so it has to come from the
    /// image rather than being assumed.
    pub fn firmware_console_type(&self) -> Option<u8> {
        self.firmware.save_bytes().and_then(|b| b.get(0x1D).copied())
    }
```

Both return `Option` — a stub or truncated firmware must degrade, not panic.

---

## 16.3 Patching the touch calibration

A real DS is calibrated by its owner tapping crosshairs; the resulting ADC
values live in the user-settings block. An emulator has no such physical
variation: screen coordinates _are_ the truth. So Lunaris rewrites the
calibration to the identity mapping at load time
([spi.rs:106-156](core/src/hw/spi.rs#L106-L156)):

```rust
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
```

```text
   calibration point 1: ADC (0, 0)              ↔ screen (0, 0)
   calibration point 2: ADC (255<<4, 191<<4)    ↔ screen (255, 191)

   The game's own calibration maths then reduces to:
        screen_x = adc_x >> 4
   which is exactly what TSC::press_screen produces (§16.4).
```

### The CRC has to be fixed too

Games and firmware validate the settings block. Patch the bytes without
updating the CRC16 and the console decides the settings are corrupt
([spi.rs:135-155](core/src/hw/spi.rs#L135-L155)):

```rust
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
```

This is the DS's own CRC16 variant, table-free, bit at a time. The `vals` array
is the polynomial expanded per bit position.

Note `mem.flush()` and then no retained handle — the same `SaveMem` discipline
as Chapter 15, so the firmware file is never locked.

---

## 16.4 The touchscreen

The TSC2046 is a channel-selecting ADC. One control byte, then two bytes of
result ([spi/tsc.rs:30-53](core/src/hw/spi/tsc.rs#L30-L53)):

```rust
    pub fn read(&self) -> u8 {
        self.return_byte
    }

    pub fn write(&mut self, value: u8) {
        self.return_byte = match self.pos {
            0 => self.value >> 5,
            1 => self.value << 3,
            _ => 0,
        } as u8;

        if value & 0x80 != 0 {
            let channel = value >> 4 & 0x7;
            self.pos = 0;
            self.value = match channel {
                1 => self.y,
                5 => self.x,
                6 => 0, // TODO: Microphone,
                _ => 0xFFF,
            };
        } else {
            self.pos += 1
        }
    }
```

```text
   control byte
    7  6  5  4  3  2  1  0
   ┌──┬────────┬──┬──┬─────┐
   │ 1│channel │MD│SR│ PD  │        bit 7 = "this is a control byte"
   └──┴────────┴──┴──┴─────┘

   channel 1 = Y position
   channel 5 = X position
   channel 6 = microphone   (returns 0 — not emulated)
   other     = 0xFFF

   transfer sequence
   ─────────────────
   write control (bit7=1) ─► latch value, pos = 0, return previous byte
   write 00h              ─► return value >> 5   (bits 11..4), pos = 1
   write 00h              ─► return value << 3   (bits 3..0 << 3), pos = 2

   12-bit result split across two bytes:
     byte0 = ▓▓▓▓▓▓▓▓ (value[11:4])
     byte1 = ▓▓▓▓0000 (value[3:0] << 3, one bit of padding)
```

Coordinates are stored pre-shifted so the ADC reading matches the calibration
patched in §16.3 ([spi/tsc.rs:59-67](core/src/hw/spi/tsc.rs#L59-L67)):

```rust
    pub fn press_screen(&mut self, x: usize, y: usize) {
        self.x = (x as u16) << 4;
        self.y = (y as u16) << 4;
    }

    pub fn release_screen(&mut self) {
        self.x = 0;
        self.y = 0xFFF;
    }
```

The release values are not zero for both axes: `(0, 0xFFF)` is the
out-of-range reading a real panel gives when nothing is touching it. Games
check for exactly that pattern.

### Pen-down is a separate signal

The ADC reading alone does not say "the pen is down". That bit lives in
EXTKEYIN, the ARM7-only key register (Chapter 17) — so a full stylus press is
two pieces of state that must be set and cleared together.

---

## 16.5 Touch latency, and why it is a frontend problem

Because a scanline of emulation runs far faster than a display frame, the point
in a frame at which the frontend injects a stylus sample changes how responsive
the pen feels.

```text
   naive frontend                       lower-latency
   ──────────────                       ─────────────
   for frame:                           for frame:
     emulate_frame()                      handle_stylus()   ◄── sample first
     handle_stylus()   ◄── one frame       emulate_frame()
                           late

   Lunaris' egui frontend queues stylus samples so that exactly one sample is
   consumed per emulated frame, rather than whatever the host mouse happened
   to be doing when the frame ended.
```

This is a frontend concern (`gui/egui`), not a core one — the core exposes only
`press_screen` / `release_screen` ([nds.rs:272-280](core/src/nds.rs#L272-L280)).

---

## 16.6 Power management

```text
   device 0: the power-management chip
     register 0  control (sound amp, backlight enable, power)
     register 1  battery status
     register 4  backlight brightness

   Lunaris: SPICNT selects it, all reads return 0, writes are dropped
   ([spi.rs:51-82](core/src/hw/spi.rs#L51-L82), the `_ => 0` / `_ => ()` arms)
```

The observable consequence is minor: a game that reads back the backlight state
sees zeros. Nothing in retail software depends on it, because a game that turns
the backlight off does not then check.

`POWCNT1` (Chapter 12) and `POWCNT2` are separate, memory-mapped registers, not
SPI devices — only the _analogue_ power chip is on the bus.

---

## 16.7 Divergences

- **Microphone is not emulated.** TSC channel 6 returns 0
  (`// TODO: Microphone`, [spi/tsc.rs:47](core/src/hw/spi/tsc.rs#L47)). Games
  with blowing/shouting mechanics (Zelda: Phantom Hourglass, Nintendogs) lose
  that input.
- **The power-management chip is a stub.**
- **SPI transfer timing is not modelled.** The busy flag is noted as
  `// TODO: Set busy flag properly` ([spi.rs:206-209](core/src/hw/spi.rs#L206-L209)),
  and the SPI_BUS completion IRQ is therefore never raised on a timer.
- **The firmware boot menu is never entered** — Lunaris always direct-boots
  (Chapter 14).
- **User settings are overwritten, not read.** A user's own nickname, colour
  and birthday from a real firmware dump survive; only the calibration block
  and its CRC are rewritten.

---

[← 15. Backup Memory and Save Files](15_backup_memory.md) | [Next: 17. RTC, Keypad and the Maths Units →](17_rtc_keypad_math.md)
