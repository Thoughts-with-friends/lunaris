# 14. The Cartridge and Boot

The cartridge is not memory-mapped. It is a **serial device** behind an 8-byte
command register, and every byte a game reads from its own ROM travels through
that protocol.

GBATEK references:
[Cartridge header](https://problemkaputt.de/gbatek.htm#dscartridgeheader) ·
[Cartridge protocol](https://problemkaputt.de/gbatek.htm#dscartridgeprotocol) ·
[Cartridge I/O ports](https://problemkaputt.de/gbatek.htm#dscartridgeioports) ·
[Secure area](https://problemkaputt.de/gbatek.htm#dscartridgesecurearea) ·
[KEY1 encryption](https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1)

---

## 14.1 The ROM file

An `.nds` file is a flat image of the cartridge.

```text
   offset      content
   ─────────   ─────────────────────────────────────────────────────────
   0000h       ┌──────────────────────────────────────┐
               │ HEADER (200h bytes)                  │  parsed by Lunaris
               │  000h game title (12 ASCII)          │
               │  00Ch game code   (4 ASCII) ────────────► backup DB lookup
               │  012h unit code, 013h encryption seed│    KEY1 key code
               │  014h device capacity                │
               │  020h arm9_rom_offset ───────────────────┐
               │  024h arm9_entry_addr                │   │
               │  028h arm9_ram_addr                  │   │  direct boot
               │  02Ch arm9_size                      │   │  uses these
               │  030h..03Fh same four for ARM7 ──────────┤
               │  040h fnt_offset / size              │   │
               │  048h fat_offset / size              │   │
               │  050h arm9 overlay table             │   │
               │  068h icon_offset                    │   │
               │  06Ch secure_area_checksum           │   │
               │  15Eh header_checksum                │   │
               └──────────────────────────────────────┘   │
   0200h       ┌──────────────────────────────────────┐   │
               │ (padding / small tables)             │   │
   4000h       ├──────────────────────────────────────┤ ◄─┘
               │ SECURE AREA (2 KB)                   │  KEY1-encrypted
               │  first 8 bytes: "encryObj" after     │  (§14.4)
               │  BIOS decryption                     │
   4800h       ├──────────────────────────────────────┤
               │ ARM9 binary (rest)                   │
               ├──────────────────────────────────────┤
               │ ARM7 binary                          │
               ├──────────────────────────────────────┤
               │ FNT — file name table (directories)  │  game-side
               │ FAT — file allocation table          │  filesystem;
               │       8 bytes/file: start, end       │  Lunaris does
               ├──────────────────────────────────────┤  not parse these
               │ overlay tables + overlay files       │
               ├──────────────────────────────────────┤
               │ banner / icon                        │
               ├──────────────────────────────────────┤
               │ FILE DATA — graphics, audio, text,   │
               │ models, scripts …                    │
               └──────────────────────────────────────┘
```

The header struct mirrors the layout field by field, in ROM order
([header.rs:11-58](core/src/hw/cartridge/header.rs#L11-L58)):

```rust
pub struct Header {
    pub game_title: [u8; 12], // ASCII
    pub game_code: u32,       // ASCII - 0 = homebrew
    pub maker_code: [u8; 2],  // ASCII - 0 = homebrew
    pub unit_code: UnitCode,
    pub encryption_seed: u8, // 0x0 - 0x7
    pub device_capacity: u8, // 0x2_0000 << nn
    // ...
    pub arm9_rom_offset: u32,
    pub arm9_entry_addr: u32, // 0x0200_0000 - 0x023B_FE00
    pub arm9_ram_addr: u32,   // 0x0200_0000 - 0x023B_FE00
    pub arm9_size: u32,       // Max 0x3BFE00
```

Only the header is Lunaris's business. FNT/FAT/overlays/NARC archives are the
game's own filesystem — see Chapter 9, §9.4.

---

## 14.2 Boot

```text
   direct boot (what Lunaris does)
   ───────────────────────────────
   HW::new(..., direct_boot = true)
        │
        ├─ Cartridge::new(rom, save_path, bios7)
        │     ├─ parse header
        │     ├─ Key1Encryption::new(bios7)      ← key table from BIOS 30h..1077h
        │     ├─ encrypt_secure_area()           ← §14.4
        │     └─ open the .sav backup            ← Chapter 15
        │
        ├─ init_arm9(): copy arm9_size bytes
        │     rom[arm9_rom_offset ..] → RAM at arm9_ram_addr
        │     write firmware setting stubs (23FFC80h = 5)
        │     return arm9_entry_addr
        │
        ├─ init_arm7(): same for the ARM7 binary
        │
        └─ RegValues::direct_boot(pc)            ← Chapter 3, §3.9
```

[mem/arm9.rs:183-196](core/src/hw/mem/arm9.rs#L183-L196):

```rust
    pub fn init_arm9(&mut self) -> u32 {
        let start_addr = self.cartridge.header().arm9_ram_addr;
        let rom_offset = self.cartridge.header().arm9_rom_offset as usize;
        let size = self.cartridge.header().arm9_size;
        for (i, addr) in (start_addr..start_addr + size).enumerate() {
            self.arm9_write(addr, self.cartridge.rom()[rom_offset + i]);
        }
        self.arm9_write(0x23FFC80, 0x5u8); // 5: firmware version
        self.cartridge.header().arm9_entry_addr
    }
```

Direct boot is _not_ the same as "the cartridge is now irrelevant". Games
stream data off the cart constantly, and some re-read their own secure area at
runtime — which is why the whole protocol below still has to work.

---

## 14.3 The transfer protocol

```text
   ┌─────────────────────────────────────────────────────────────────────┐
   │ 1. write 8 command bytes  ──►  ROMCMD   4001A8h..4001AFh            │
   │ 2. write ROMCTRL with block size, set bit 31 (Start/Busy)  4001A4h  │
   │ 3. hardware streams words:                                          │
   │       Event::ROMWordTransfered ─► data_word_ready = 1               │
   │                                 ─► triggers DSCartridge DMA          │
   │       CPU (or DMA) reads ROMDATA 4100010h                           │
   │       …repeat…                                                      │
   │ 4. Event::ROMBlockEnded ─► block_busy = 0                           │
   │                         ─► GAME_CARD_TRANSFER_COMPLETION IRQ        │
   └─────────────────────────────────────────────────────────────────────┘
```

Block size is a 3-bit field with a non-linear encoding
([cartridge.rs:210-252](core/src/hw/cartridge.rs#L210-L252)):

```rust
    pub fn run_command(&mut self, scheduler: &mut Scheduler, is_arm9: bool) {
        assert_eq!(self.rom_bytes_left % 4, 0);
        self.rom_bytes_left = match self.romctrl.data_block_size {
            0 => 0,
            7 => 4,
            _ => {
                assert!(self.romctrl.data_block_size < 7);
                0x100 << self.romctrl.data_block_size
            }
        };
        self.romctrl.block_busy = true;
        self.romctrl.data_word_ready = false;
        self.game_card_words.clear();

        if self.key1_encryption.in_use {
            self.run_encrypted_command();
        } else {
            self.run_unencrypted_command();
        }
```

```text
   data_block_size   bytes
   ───────────────   ─────────
        0            0        (command only, no data)
        1            0x200
        2            0x400
        3            0x800
        4            0x1000
        5            0x2000
        6            0x4000
        7            4        ← the odd one out
```

Lunaris pushes the entire block into `game_card_words` up front, then meters it
out one word per scheduled event
([cartridge.rs:484-492](core/src/hw/cartridge.rs#L484-L492)):

```rust
    pub fn on_rom_word_transfered(&mut self, event: Event) {
        let is_arm9 = match event {
            Event::ROMWordTransfered(is_arm9) => is_arm9,
            _ => unreachable!(),
        };
        self.cartridge.cur_game_card_word = self.cartridge.game_card_words.pop_front().unwrap();
        self.cartridge.romctrl.data_word_ready = true;
        self.run_dmas_single(dma::Occasion::DSCartridge, is_arm9);
    }
```

That is the whole cartridge-DMA mechanism: one word ready → run the armed
DSCartridge DMA channel → it reads ROMDATA once.

Timing is derived from the command byte time
([cartridge.rs:236-252](core/src/hw/cartridge.rs#L236-L252)):

```rust
        if self.rom_bytes_left == 0 {
            // 8 command bytes transferred
            scheduler.schedule(
                Event::ROMBlockEnded(is_arm9),
                HW::on_rom_block_ended,
                self.transfer_byte_time() * 8,
            );
        } else {
            // 8 command bytes + 4 bytes for word
            scheduler.schedule(
                Event::ROMWordTransfered(is_arm9),
                HW::on_rom_word_transfered,
                self.transfer_byte_time() * (8 + 4),
            );
        }
```

### The command set

[cartridge.rs:310-360](core/src/hw/cartridge.rs#L310-L360):

```rust
    pub fn run_unencrypted_command(&mut self) {
        match self.command[0] {
            0x00 => { /* read header, first rom_bytes_left bytes */ }
            0x3C => {
                self.key1_encryption.init_key_code(self.header.game_code, 2, 2);
            }
            0xB7 => {
                let addr = u32::from_be_bytes(self.command[1..=4].try_into().unwrap()) as usize;
                self.push_rom_data(addr, self.rom_bytes_left);
            }
            0xB8 => { /* chip ID, repeated */ }
            0x90 => { /* chip ID, repeated */ }
            0x9F => { /* endless HIGH-Z: 0xFFFF_FFFF */ }
            _ => {
                warn!("Unimplemented Unencrypted Cartridge Command: {:X?}", self.command);
                for _ in 0..self.rom_bytes_left / 4 {
                    self.game_card_words.push_back(0);
                }
            }
        };
    }
```

```text
   00h   read header (0..1000h)
   3Ch   activate KEY1 encryption           ← flips in_use; everything after
                                              this goes through the encrypted
                                              command path
   B7h   read ROM data at a 32-bit address  ← the workhorse; big-endian!
   B8h   get chip ID
   90h   get chip ID (KEY1 idle equivalent)
   9Fh   dummy / high-Z
```

Note `from_be_bytes` on the B7h address. Cartridge commands are **big-endian**
while the rest of the DS is little-endian — a byte-order mistake here reads
completely the wrong part of the ROM.

### Address normalisation

Two hardware behaviours, both easy to miss
([cartridge.rs:363-393](core/src/hw/cartridge.rs#L363-L393)):

```rust
    fn normalize_rom_addr(&self, addr: usize) -> usize {
        // Cartridge ROM mirrors when exceeding ROM size
        let mut addr = addr % self.rom.len();

        // Addresses below 0x8000 are redirected
        // GBATEK:
        //   0x8000 + (addr & 0x1FF)
        if addr < 0x8000 {
            addr = 0x8000 + (addr & 0x1FF);
        }

        addr
    }

    fn push_rom_data(&mut self, base_addr: usize, len: usize) {
        assert_eq!(len % 4, 0);

        // DS cartridges wrap within current 4KB block
        let block_start = base_addr & !0xFFF;

        for offset in (0..len).step_by(4) {
            // Wrap inside current 4KB block
            let wrapped_addr = block_start | ((base_addr + offset) & 0xFFF);
            let addr = self.normalize_rom_addr(wrapped_addr);
            // ...
```

```text
   1. Reads below 8000h are REDIRECTED, not blocked:
      addr 0x1234  →  0x8000 + (0x1234 & 0x1FF) = 0x8034
      This is what protects the secure area from being read normally.

   2. A read WRAPS INSIDE ITS 4 KB BLOCK:
      base 0x00FF8, len 0x200
        →  0x00FF8, 0x00FFC, then back to 0x00000, 0x00004, …
      not 0x01000, 0x01004.
```

---

## 14.4 KEY1 and the secure area

KEY1 is Blowfish, keyed from a table inside the ARM7 BIOS plus the game code
([key1_encryption.rs:14-50](core/src/hw/cartridge/key1_encryption.rs#L14-L50)):

```rust
pub struct Key1Encryption {
    pub in_use: bool,
    key_buf: [u32; Self::KEY_TABLE_SIZE],
    original_key_buf: [u32; Self::KEY_TABLE_SIZE],
}

impl Key1Encryption {
    const KEY_TABLE_SIZE: usize = 0x1048 / 4;

    pub fn new(bios7: &[u8]) -> Self {
        let original_key_buf: [u32; Self::KEY_TABLE_SIZE] =
            bytemuck::cast_slice(&bios7[0x30..=0x1077]).try_into().unwrap();

        Self { in_use: false, key_buf: original_key_buf, original_key_buf }
    }

    pub fn init_key_code(&mut self, id_code: u32, level: u32, modulo: u32) {
        self.in_use = true;
        self.key_buf = self.original_key_buf;

        let mut key_code = [id_code, id_code / 2, id_code * 2];

        if level >= 1 { self.apply_keycode(&mut key_code, modulo); }
        if level >= 2 { self.apply_keycode(&mut key_code, modulo); }
        if level >= 3 {
            key_code[1] *= 2;
            key_code[2] /= 2;
            self.apply_keycode(&mut key_code, modulo);
        }
```

Keeping `original_key_buf` alongside the working copy matters: `init_key_code`
is called repeatedly at different levels, and each call must start from the
pristine BIOS table.

### The re-encryption problem

```text
   A cartridge as manufactured        A ROM dumped from a running console
   ────────────────────────────        ───────────────────────────────────
   4000h ┌─────────────────┐           4000h ┌─────────────────┐
         │ "encryObj" (L2  │                 │ E7FFDEFF …      │  ← "destroyed"
         │  + L3 encrypted)│                 │ (BIOS overwrote │     marker
         ├─────────────────┤                 │  the first 8 B) │
         │ secure area,    │                 ├─────────────────┤
         │ L3 encrypted    │                 │ secure area,    │
         └─────────────────┘                 │ PLAIN TEXT      │
                                             └─────────────────┘
                                                     │
                              encrypt_secure_area() restores the left-hand form
```

Most dumped ROMs are in the right-hand state, so Lunaris **re-encrypts** at load
time ([cartridge.rs:168-201](core/src/hw/cartridge.rs#L168-L201)):

```rust
    pub fn encrypt_secure_area(&mut self) {
        let start = self.header.arm9_rom_offset as usize;
        if !Self::SECURE_AREA_RANGE.contains(&start) {
            return;
        }
        // ... verify the "destroyed secure area" marker ...

        // First 8 bytes over secure area is overwritten by BIOS after decryption, so put correct string
        secure_area[..0x8].copy_from_slice(Self::DECRYPTED_SECURE_AREA_ID.as_bytes());
        let secure_area_32: &mut [u32] =
            bytemuck::cast_slice_mut(&mut self.rom[secure_area_range()]);
        // Level 3 for entire secure area
        self.key1_encryption.init_key_code(self.header.game_code, 3, 2);
        for chunk in secure_area_32.chunks_exact_mut(2) {
            self.key1_encryption.encrypt(chunk);
        }
        // Level 2 for first 8 bytes (first 8 bytes encrypted twice)
        self.key1_encryption.init_key_code(self.header.game_code, 2, 2);
        self.key1_encryption.encrypt(&mut secure_area_32[..0x8]);
        self.key1_encryption.in_use = false;
    }
```

The double encryption of the first 8 bytes (level 3 then level 2) is exactly
what hardware expects; a game that re-reads its secure area and checks the
`"encryObj"` string will refuse to run otherwise.

Every step of the function bails out rather than asserting if the ROM is not in
the expected form — an already-encrypted ROM, or a homebrew with no secure area
at all, simply passes through untouched.

---

## 14.5 The ROM is not in the savestate

[cartridge.rs:59-68](core/src/hw/cartridge.rs#L59-L68):

```rust
    /// Not serialized: ROMs are immutable and can be tens to hundreds of MB,
    /// which used to make every savestate file balloon to the ROM's full
    /// size for no benefit. The ROM is re-supplied by the host at
    /// [`Cartridge::new`] time instead; savestate files carry only a small
    /// fingerprint (see `NDS::rom_fingerprint`) so a mismatched ROM can be
    /// rejected before a load is applied.
    #[savestate(skip)]
    rom: Vec<u8>,
```

This one change took savestates from ~137 MB to a couple of megabytes. The
fingerprint that replaces it is game code + the two header checksums + ROM
length ([hw.rs:577-590](core/src/hw.rs#L577-L590)) — enough to catch "you
loaded this state against a different ROM" without hashing 128 MB every time.

Chapter 19 covers the rest of the savestate design.

---

## 14.6 Divergences

- **Chip ID is a constant.** `// TODO: derive from ROM size and manufacturer
code` ([cartridge.rs:55-57](core/src/hw/cartridge.rs#L55-L57)). Games that
  validate the chip ID against the header's device capacity would notice.
- **KEY2 is not implemented.** The ROMCTRL KEY2 flags are stored and ignored.
  KEY2 is a stream cipher applied on top of KEY1 for later commands; real
  hardware needs it, but since Lunaris _is_ both ends of the bus, skipping it is
  invisible.
- **Command 00h asserts a block size below 0x1000** (`// TODO: Support`).
- **Unimplemented commands return zeros** with a `warn!` rather than failing.
- **Transfer timing** is approximated from the ROMCTRL clock rate; per-command
  gap timings (KEY1 gap1/gap2) are parsed but not applied.

---

[← 13. The Sound Processing Unit](13_spu.md) | [Next: 15. Backup Memory and Save Files →](15_backup_memory.md)
