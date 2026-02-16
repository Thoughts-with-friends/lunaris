/// Touch screen controller emulation.
///
/// This struct emulates the Nintendo DS touchscreen SPI device.
/// It faithfully reproduces the behavior of the original C++
/// implementation from CorgiDS.
#[derive(Debug, Default)]
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
                1 => self.output_coords = self.press_y, // Touch Y
                5 => self.output_coords = self.press_x, // Touch X
                6 => self.output_coords = 0x800,        // Battery / auxiliary channel
                _ => self.output_coords = 0xFFF,
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
    #[expect(unused)]
    pub fn deselect(&mut self) {
        self.data_pos = 0;
    }
}
