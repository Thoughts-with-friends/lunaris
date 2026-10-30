//! KEY2 seed registers (ROMSEED0/1), for the stream cipher on the data
//! bus. See GBATEK "DS Encryption by Random Seed (KEY2)".
use crate::NDSCart;

impl NDSCart {
    /// Sets low 32 bits of KEY2 seed 0.
    pub const fn set_lo_key2_seed0(&mut self, word: u32) {
        self.encrypt_seed0 >>= 32;
        self.encrypt_seed0 <<= 32;
        self.encrypt_seed0 |= word as u64;
    }

    /// Sets low 32 bits of KEY2 seed 1.
    pub const fn set_lo_key2_seed1(&mut self, word: u32) {
        self.encrypt_seed1 >>= 32;
        self.encrypt_seed1 <<= 32;
        self.encrypt_seed1 |= word as u64;
    }

    /// Sets high 32 bits of KEY2 seed 0.
    pub const fn set_hi_key2_seed0(&mut self, word: u32) {
        // DS Encryption by Random Seed (KEY2): https://problemkaputt.de/gbatek.htm#dsencryptionbyrandomseedkey2
        let word = word & 0x7F; // seed upper bits (bits 0-6)
        self.encrypt_seed0 <<= 32;
        self.encrypt_seed0 >>= 32;
        self.encrypt_seed0 |= (word as u64) << 32;
    }

    /// Sets high 32 bits of KEY2 seed 1.
    pub const fn set_hi_key2_seed1(&mut self, word: u32) {
        let word = word & 0x7F; // seed upper bits (bits 0-6)
        self.encrypt_seed1 <<= 32;
        self.encrypt_seed1 >>= 32;
        self.encrypt_seed1 |= (word as u64) << 32;
    }
}
