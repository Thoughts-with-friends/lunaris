use std::convert::TryInto;

#[derive(emu_utils::Savestate)]
pub struct Key1Encryption {
    pub in_use: bool,
    key_buf: [u32; Self::KEY_TABLE_SIZE],
    original_key_buf: [u32; Self::KEY_TABLE_SIZE],
}

impl Key1Encryption {
    const KEY_TABLE_SIZE: usize = 0x1048 / 4;

    const P_ARRAY_END: usize = 0x44 / 4;

    pub fn new(bios7: &[u8]) -> Self {
        let original_key_buf: [u32; Self::KEY_TABLE_SIZE] =
            bytemuck::cast_slice(&bios7[0x30..=0x1077]).try_into().unwrap();

        Self { in_use: false, key_buf: original_key_buf, original_key_buf }
    }

    pub fn init_key_code(&mut self, id_code: u32, level: u32, modulo: u32) {
        self.in_use = true;
        self.key_buf = self.original_key_buf;

        let mut key_code = [id_code, id_code / 2, id_code * 2];

        if level >= 1 {
            self.apply_keycode(&mut key_code, modulo);
        }

        if level >= 2 {
            self.apply_keycode(&mut key_code, modulo);
        }

        if level >= 3 {
            key_code[1] *= 2;
            key_code[2] /= 2;
            self.apply_keycode(&mut key_code, modulo);
        }
    }

    pub fn encrypt(&self, ptr: &mut [u32]) {
        self.crypt::<true>(ptr)
    }

    pub fn decrypt(&self, ptr: &mut [u32]) {
        self.crypt::<false>(ptr)
    }

    fn crypt<const ENCRYPT: bool>(&self, ptr: &mut [u32]) {
        let block: &mut [u32; 2] = ptr.try_into().unwrap();

        let [mut y, mut x] = *block;

        if ENCRYPT {
            for i in 0x0..=0xF {
                round(&self.key_buf, i, &mut x, &mut y);
            }

            block[0] = x ^ self.key_buf[0x10];
            block[1] = y ^ self.key_buf[0x11];
        } else {
            for i in (0x2..=0x11).rev() {
                round(&self.key_buf, i, &mut x, &mut y);
            }

            block[0] = x ^ self.key_buf[0x1];
            block[1] = y ^ self.key_buf[0x0];
        }
    }

    fn apply_keycode(&mut self, key_code: &mut [u32; 3], modulo: u32) {
        self.encrypt_words(&mut key_code[1..3]);
        self.encrypt_words(&mut key_code[0..2]);

        for i in 0..=Self::P_ARRAY_END {
            self.key_buf[i] ^= key_code[i % modulo as usize].swap_bytes();
        }

        let mut scratch = [0, 0];

        for i in (0..Self::KEY_TABLE_SIZE).step_by(2) {
            self.encrypt(&mut scratch);

            self.key_buf[i] = scratch[1];
            self.key_buf[i + 1] = scratch[0];
        }
    }

    fn encrypt_words(&self, words: &mut [u32]) {
        let block: &mut [u32; 2] = words.try_into().unwrap();
        self.encrypt(block);
    }
}

#[inline(always)]
fn round(key_buf: &[u32], i: usize, x: &mut u32, y: &mut u32) {
    let z = (key_buf[i] ^ *x) as usize;

    const S_BOX0: usize = 0x048 / 4;
    const S_BOX1: usize = 0x448 / 4;
    const S_BOX2: usize = 0x848 / 4;
    const S_BOX3: usize = 0xC48 / 4;

    let mut f = key_buf[S_BOX0 + ((z >> 24) & 0xFF)];
    f = f.wrapping_add(key_buf[S_BOX1 + ((z >> 16) & 0xFF)]);
    f ^= key_buf[S_BOX2 + ((z >> 8) & 0xFF)];
    f = f.wrapping_add(key_buf[S_BOX3 + (z & 0xFF)]);

    *x = f ^ *y;
    *y = z as u32;
}
