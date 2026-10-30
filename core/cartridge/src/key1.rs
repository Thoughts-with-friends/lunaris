//! KEY1 encryption (Blowfish variant keyed from the ARM7 BIOS), used for
//! the secure area commands. See GBATEK "DS Encryption by Gamecode/Idcode
//! (KEY1)".
use crate::NDSCart;

impl NDSCart {
    /// Word `index` of the KEY1 key buffer.
    #[inline]
    fn read_keybuf_u32(&self, index: usize) -> u32 {
        let i = index * 4;
        u32::from_le_bytes([
            self.key1_buffer[i],
            self.key1_buffer[i + 1],
            self.key1_buffer[i + 2],
            self.key1_buffer[i + 3],
        ])
    }

    /// Stores word `index` of the KEY1 key buffer.
    #[inline]
    fn write_keybuf_u32(&mut self, index: usize, value: u32) {
        let i = index * 4;
        let b = value.to_le_bytes();
        self.key1_buffer[i] = b[0];
        self.key1_buffer[i + 1] = b[1];
        self.key1_buffer[i + 2] = b[2];
        self.key1_buffer[i + 3] = b[3];
    }

    /// Performs KEY1 encryption on data buffer.
    #[must_use]
    pub fn key1_encrypt(&mut self, mut y: u32, mut x: u32) -> [u32; 2] {
        // #[cfg(feature = "log")]
        // log::info!(?y, ?x);

        // DS Encryption by Gamecode/Idcode (KEY1): https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1
        for i in 0..=0xF {
            // 16 Blowfish rounds
            let z = (self.read_keybuf_u32(i) ^ x) as usize;
            x = self.read_keybuf_u32(0x012 + ((z >> 24) & 0xFF)); // S-box 0 at keybuf 012h words, index = bits 24-31
            x = x.wrapping_add(self.read_keybuf_u32(0x112 + ((z >> 16) & 0xFF))); // S-box 1 at keybuf 112h words, index = bits 16-23
            x ^= self.read_keybuf_u32(0x212 + ((z >> 8) & 0xFF)); // S-box 2 at keybuf 212h words, index = bits 8-15
            x = x.wrapping_add(self.read_keybuf_u32(0x312 + (z & 0xFF))); // S-box 3 at keybuf 312h words, index = bits 0-7
            x ^= y;
            y = z as u32;
        }

        [
            x ^ self.read_keybuf_u32(0x10), // P-array entry 10h
            y ^ self.read_keybuf_u32(0x11), // P-array entry 11h
        ]
    }

    /// Performs KEY1 decryption on data buffer.
    pub(crate) fn key1_decrypt(&self, data: &mut [u32]) {
        let mut y = data[0];
        let mut x = data[1];

        for i in (0x2..=0x11).rev() {
            // decrypt walks the P-array backwards (11h..02h)
            let z = self.read_keybuf_u32(i) ^ x;

            x = self.read_keybuf_u32(0x012 + ((z >> 24) as usize & 0xFF)); // S-box 0, index = bits 24-31
            x = x.wrapping_add(self.read_keybuf_u32(0x112 + ((z >> 16) as usize & 0xFF))); // S-box 1, index = bits 16-23
            x ^= self.read_keybuf_u32(0x212 + ((z >> 8) as usize & 0xFF)); // S-box 2, index = bits 8-15
            x = x.wrapping_add(self.read_keybuf_u32(0x312 + (z as usize & 0xFF))); // S-box 3, index = bits 0-7
            x ^= y;

            y = z;
        }

        data[0] = x ^ self.read_keybuf_u32(1);
        data[1] = y ^ self.read_keybuf_u32(0);
    }

    /// Applies keycode transformation.
    pub fn apply_keycode(&mut self, modulo: u32) {
        {
            let y = self.keycode[1];
            let x = self.keycode[2];
            let data = self.key1_encrypt(y, x);
            self.keycode[1] = data[0];
            self.keycode[2] = data[1];
        }
        {
            let y = self.keycode[0];
            let x = self.keycode[1];
            let data = self.key1_encrypt(y, x);
            self.keycode[0] = data[0];
            self.keycode[1] = data[1];
        }

        for i in 0..0x11 {
            // 18 P-array words
            let mut value = self.read_keybuf_u32(i);
            value ^= byteswap_word(self.keycode[i % modulo as usize]);
            self.write_keybuf_u32(i, value);
        }

        let mut scratch = [0; 2];
        for i in (0..=0x410).skip(1) {
            // 412h words of P-array + S-boxes
            {
                let y = scratch[0];
                let x = scratch[1];
                let data = self.key1_encrypt(y, x);
                scratch[0] = data[0];
                scratch[1] = data[1];
            }

            self.write_keybuf_u32(i, scratch[1]);
            self.write_keybuf_u32(i + 1, scratch[0]);
        }
    }
}

/// Reverses the byte order of a word.
const fn byteswap_word(word: u32) -> u32 {
    let mut result = 0;
    result |= word >> 24;
    result |= (word & 0xFF0000) >> 8; // byte swap: byte 2 -> byte 1
    result |= (word & 0xFF00) << 8; // byte swap: byte 1 -> byte 2
    result |= word << 24; // byte 0 -> byte 3
    result
}
