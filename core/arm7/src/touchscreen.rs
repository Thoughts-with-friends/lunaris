/// Touch screen controller emulation.
///
/// This struct emulates the Nintendo DS touchscreen SPI device.
/// It faithfully reproduces the behavior of the original C++
/// implementation from CorgiDS.
pub struct TouchScreen {
    // Control byte written by SPI
    control_byte: u8,

    // Output coordinate buffer (12-bit ADC result)
    output_coords: u16,

    // Current byte position during SPI transfer
    data_pos: i32,

    // Latched touch coordinates
    press_x: u16,
    press_y: u16,
}

impl TouchScreen {
    /// Create a new TouchScreen instance.
    ///
    /// Initializes all internal state exactly as the original
    /// C++ constructor.
    pub fn new() -> Self {
        Self {
            data_pos: 0,
            control_byte: 0,
            output_coords: 0,
            press_x: 0,
            press_y: 0xFFF,
        }
    }

    /// Power on (or reset) the touchscreen device.
    ///
    /// This resets all internal state and latched coordinates,
    /// matching the original C++ `power_on()` behavior.
    pub fn power_on(&mut self) {
        self.data_pos = 0;
        self.control_byte = 0;
        self.output_coords = 0;

        self.press_x = 0;
        self.press_y = 0xFFF;
    }

    /// Register a touch press event.
    ///
    /// The coordinates are latched internally and converted
    /// to 12-bit ADC values by shifting left by 4 bits,
    /// exactly like the original implementation.
    ///
    /// A Y value of `0xFFF` indicates "no touch".
    pub fn press_event(&mut self, x: i32, y: i32) {
        self.press_x = x as u16;
        self.press_y = y as u16;

        if y == 0xFFF {
            return;
        }

        self.press_x <<= 4;
        self.press_y <<= 4;

        // printf("\nTouchscreen: ($%04X, $%04X)", press_x, press_y);
    }

    /// Transfer one byte over the touchscreen SPI interface.
    ///
    /// This function emulates the exact SPI behavior of the
    /// touchscreen controller, including:
    /// - Control byte decoding
    /// - Channel selection
    /// - ADC result shifting
    /// - Data position tracking
    pub fn transfer_data(&mut self, input: u8) -> u8 {
        // Output previously prepared data
        let data = if self.data_pos == 0 {
            ((self.output_coords >> 5) & 0xFF) as u8
        } else if self.data_pos == 1 {
            ((self.output_coords << 3) & 0xFF) as u8
        } else {
            0
        };

        if (input & (1 << 7)) != 0 {
            // Set control byte
            self.control_byte = input;
            let channel = (self.control_byte >> 4) & 0x7;
            self.data_pos = 0;

            // Select ADC channel
            match channel {
                1 => {
                    // Touch Y
                    self.output_coords = self.press_y;
                }
                5 => {
                    // Touch X
                    self.output_coords = self.press_x;
                }
                6 => {
                    // Battery / auxiliary channel
                    self.output_coords = 0x800;
                }
                _ => {
                    self.output_coords = 0xFFF;
                }
            }

            // Conversion mode change
            if (self.control_byte & 0x8) != 0 {
                self.output_coords &= 0x0FF0;
            }
        } else {
            self.data_pos += 1;
        }

        data
    }

    /// Deselect the touchscreen SPI device.
    ///
    /// This resets the internal transfer position, exactly
    /// like the original C++ `deselect()` method.
    pub fn deselect(&mut self) {
        self.data_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::TouchScreen;

    /// Test initial power-on state.
    ///
    /// Ensures internal state matches the C++ constructor and power_on().
    #[test]
    fn test_power_on_state() {
        let mut ts = TouchScreen::new();
        ts.power_on();

        assert_eq!(ts.press_x, 0);
        assert_eq!(ts.press_y, 0xFFF);
        assert_eq!(ts.control_byte, 0);
        assert_eq!(ts.output_coords, 0);
        assert_eq!(ts.data_pos, 0);
    }

    /// Test press event coordinate scaling.
    ///
    /// Coordinates must be shifted left by 4 bits unless y == 0xFFF.
    #[test]
    fn test_press_event_scaling() {
        let mut ts = TouchScreen::new();
        ts.press_event(100, 200);

        assert_eq!(ts.press_x, 100 << 4);
        assert_eq!(ts.press_y, 200 << 4);
    }

    /// Test no-touch press event behavior.
    ///
    /// A Y value of 0xFFF indicates no touch and must not apply shifting.
    #[test]
    fn test_press_event_no_touch() {
        let mut ts = TouchScreen::new();
        ts.press_event(123, 0xFFF);

        assert_eq!(ts.press_x, 123);
        assert_eq!(ts.press_y, 0xFFF);
    }

    /// Test SPI transfer for Touch Y channel.
    ///
    /// Verifies correct channel selection and two-byte output sequence.
    #[test]
    fn test_transfer_touch_y() {
        let mut ts = TouchScreen::new();
        ts.press_event(10, 20);

        // Control byte: start bit + channel 1 (Y)
        let control = 0b1001_0000;
        let first = ts.transfer_data(control);
        let second = ts.transfer_data(0x00);

        let expected = (20 << 4) as u16;

        assert_eq!(first, ((expected >> 5) & 0xFF) as u8);
        assert_eq!(second, ((expected << 3) & 0xFF) as u8);
    }

    /// Test SPI transfer for Touch X channel.
    #[test]
    fn test_transfer_touch_x() {
        let mut ts = TouchScreen::new();
        ts.press_event(15, 25);

        // Control byte: start bit + channel 5 (X)
        let control = 0b1101_0000;
        let first = ts.transfer_data(control);
        let second = ts.transfer_data(0x00);

        let expected = (15 << 4) as u16;

        assert_eq!(first, ((expected >> 5) & 0xFF) as u8);
        assert_eq!(second, ((expected << 3) & 0xFF) as u8);
    }

    /// Test auxiliary channel (channel 6).
    ///
    /// Must always return fixed value 0x800.
    #[test]
    fn test_transfer_aux_channel() {
        let mut ts = TouchScreen::new();

        // Control byte: start bit + channel 6
        let control = 0b1110_0000;
        let first = ts.transfer_data(control);
        let second = ts.transfer_data(0x00);

        let expected = 0x800u16;

        assert_eq!(first, ((expected >> 5) & 0xFF) as u8);
        assert_eq!(second, ((expected << 3) & 0xFF) as u8);
    }

    /// Test conversion mode masking.
    ///
    /// When control_byte & 0x8 is set, lower 4 bits must be cleared.
    #[test]
    fn test_conversion_mode_mask() {
        let mut ts = TouchScreen::new();
        ts.press_event(7, 9);

        // Control byte: start + channel 1 + conversion mode bit
        let control = 0b1001_1000;
        ts.transfer_data(control);

        assert_eq!(ts.output_coords & 0x000F, 0);
    }

    /// Test data_pos increment and overflow behavior.
    #[test]
    fn test_data_pos_progression() {
        let mut ts = TouchScreen::new();
        ts.press_event(5, 5);

        let control = 0b1001_0000;
        ts.transfer_data(control);

        assert_eq!(ts.data_pos, 0);

        ts.transfer_data(0);
        assert_eq!(ts.data_pos, 1);

        ts.transfer_data(0);
        assert_eq!(ts.data_pos, 2);
    }

    /// Test deselect behavior.
    ///
    /// data_pos must be reset to zero.
    #[test]
    fn test_deselect() {
        let mut ts = TouchScreen::new();
        ts.press_event(1, 1);

        ts.transfer_data(0b1001_0000);
        ts.transfer_data(0);
        assert!(ts.data_pos > 0);

        ts.deselect();
        assert_eq!(ts.data_pos, 0);
    }
}
