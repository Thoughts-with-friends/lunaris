# 15. Backup Memory and Save Files

The save chip is a small serial EEPROM or Flash device sitting on the AUXSPI
bus. Emulating it correctly is what separates "the game runs" from "the game is
playable", and it has more sharp edges than its size suggests.

GBATEK reference:
[DS cartridge backup](https://problemkaputt.de/gbatek.htm#dscartridgebackup) ·
[GBA EEPROM (protocol ancestry)](https://problemkaputt.de/gbatek.htm#gbacartbackupeeprom)

---

## 15.1 The problem: the chip type is not discoverable

A DS cartridge does not tell you what save chip it has. There is no register to
read, no ID command that distinguishes an 8 KB EEPROM from a 512 KB Flash.
Every emulator solves this the same way: **a database keyed by game code**.

```text
   header.game_code  ("IPKJ" → 0x4A4B5049)
            │
            ▼
   GAME_DB: 6774 entries, one per known retail game
            │
            ├─ found     → sram_type
            └─ not found → guess from an existing .sav file's size,
                           else fall back to 512 KB flash
            │
            ▼
   SRAM_SIZES[sram_type]
            │
            ▼
   Box<dyn Backup>
```

[game_db.rs:17-31](core/src/hw/cartridge/backup/game_db.rs#L17-L31):

```rust
impl dyn Backup {
    // TODO: Don't hardcode size - Fixed with better const fn
    pub const GAME_DB: [GameInfo; 6774] = <dyn Backup>::gen_game_db();

    pub const SRAM_SIZES: [usize; 10] = [
        0,
        0x200,          // 512 B     EEPROM small
        8 * 0x400,      // 8 KB      EEPROM
        64 * 0x400,     // 64 KB     EEPROM
        128 * 0x400,    // 128 KB    EEPROM large (24-bit addressing)
        256 * 0x400,    // 256 KB    Flash
        512 * 0x400,    // 512 KB    Flash
        0x10_0000,      // 1 MB      Flash
        8 * 0x10_0000,  // 8 MB      NAND
        32 * 0x10_0000, // 32 MB     NAND
    ];
```

The table is a `const fn` — 6774 entries baked into the binary at compile time,
sourced from melonDS ([game_db.rs:34-36](core/src/hw/cartridge/backup/game_db.rs#L34-L36)).

---

## 15.2 Chip selection

[backup.rs:340-396](core/src/hw/cartridge/backup.rs#L340-L396):

```rust
    pub fn detect_type(header: &Header, save_path: PathBuf) -> Box<dyn Backup> {
        let sram_type = <dyn Backup>::GAME_DB
            .iter()
            .find(|game_info| game_info.game_code == header.game_code)
            .map(|game_info| game_info.sram_type)
            .unwrap_or_else(|| {
                let guessed = Self::guess_sram_type_from_existing_file(&save_path);
                warn!(
                    target: "nds_core::savedata",
                    "Game not found in DB! Guessed sram_type=0x{guessed:X}"
                );
                guessed
            });

        // Game codes starting with ASCII `I` route their flash chip through
        // an intermediary IR MCU (Pokémon HeartGold/SoulSilver, Black/White,
        // …); the first byte of the little-endian `game_code` is the
        // cartridge header's first ASCII game-code character.
        let is_ir_cart =
            (header.game_code & 0xFF) as u8 == b'I' && std::env::var_os("LUNARIS_NO_IR").is_none();

        match sram_type {
            1 => Box::new(EEPROM::<EEPROMSmall>::new(save_path, <dyn Backup>::SRAM_SIZES[sram_type])),
            2..=3 => Box::new(EEPROM::<EEPROMNormal>::new(save_path, <dyn Backup>::SRAM_SIZES[sram_type])),
            // 128K EEPROM uses a 24-bit (3-byte) address bus, unlike the
            // 16-bit bus shared by the 8K/64K variants above.
            4 => Box::new(EEPROM::<EEPROMLarge>::new(save_path, <dyn Backup>::SRAM_SIZES[sram_type])),
            5..=7 => {
                let flash = Flash::new_backup(save_path, <dyn Backup>::SRAM_SIZES[sram_type]);
                if is_ir_cart { Box::new(IrBackup::new(flash)) } else { Box::new(flash) }
            }
            8..=9 => { /* NAND — unsupported, see §15.6 */ Box::new(NoBackup::new()) }
            sram_type => { /* 0 or 0xFFFFFFFF sentinel */ Box::new(NoBackup::new()) }
        }
    }
```

Three defensive decisions in one function, each documented above it
([backup.rs:320-339](core/src/hw/cartridge/backup.rs#L320-L339)):

```text
   sram_type 0            →  NoBackup     "the game genuinely has no save"
   sram_type 0xFFFFFFFF   →  NoBackup     sentinel in some DB rows;
                                          indexing SRAM_SIZES would PANIC
   sram_type 8..=9        →  NoBackup     NAND: accessed via ROM commands,
                                          not SPI. Routing it to Flash would
                                          produce a save the game cannot read
   game code not in DB    →  guess from an existing .sav size, else 512 KB
```

There is even a regression test for the sentinel case
([backup.rs:499-508](core/src/hw/cartridge/backup.rs#L499-L508)):
`detect_type_does_not_panic_on_sentinel_sram_types`.

---

## 15.3 The `Backup` trait

Everything the rest of the emulator needs from a save chip
([backup.rs:33-69](core/src/hw/cartridge/backup.rs#L33-L69)):

```rust
pub trait Backup {
    fn read(&self) -> u8;
    fn write(&mut self, hold: bool, value: u8);

    /// Captures the chip's in-flight SPI protocol state ...
    fn protocol_snapshot(&self) -> BackupProtocolState;
    fn restore_protocol_state(&mut self, state: BackupProtocolState);

    /// Returns the chip's persistent memory contents (the `.sav` payload).
    fn save_bytes(&self) -> Option<&[u8]>;
    fn set_save_bytes(&mut self, bytes: &[u8]);

    /// Flushes any pending writes to the `.sav` file on disk. ...
    fn flush(&mut self);
}
```

Note the split: **protocol state** and **memory contents** are separate. That
separation is the fix for a real freeze bug, covered in §15.5.

`write(hold, value)` — `hold` is the SPI chip-select. `hold == false` means the
transaction just ended, which is when the chip commits.

---

## 15.4 The SPI protocols

```text
   EEPROM (8 KB / 64 KB, 16-bit address)

   /CS ▔▔▔╲___________________________________________╱▔▔▔
   MOSI    │ 03h  │ AddrHi │ AddrLo │  --  │  --  │
   MISO    │  --  │   --   │   --   │ D[0] │ D[1] │ …
           └ READ   └───── 2 address bytes ──┘  └ streams until /CS

   EEPROM large (128 KB) — THREE address bytes, 24-bit bus

   Flash (256 KB – 1 MB)

   MOSI    │ 0Bh  │ A[23:16] │ A[15:8] │ A[7:0] │ dummy │ -- │
   MISO    │                                            │ D0 │ D1 …
           └ READ (fast)

   Common opcodes
     06h WREN  set write-enable latch      (required before any write)
     04h WRDI  clear it
     05h RDSR  read status register
     02h PP    page program
     0Ah PW    page write
     9Fh RDID  read JEDEC ID
```

Each chip is a small state machine over `(mode, address bytes seen, value)`.
That triple is exactly what the savestate captures.

---

## 15.5 Two bugs the design solves

### (a) The freeze after Load State

[backup.rs:71-79](core/src/hw/cartridge/backup.rs#L71-L79):

```rust
/// Snapshot of a backup chip's SPI protocol state machine.
///
/// Without this, a savestate captured while a game is mid-transaction with
/// its save chip (a very common window, since games poll their save chip
/// frequently) would resume with the chip reset to idle. The ARM7 would
/// then wait forever for a response to a transaction the chip never
/// started, hanging the game after a Load State even though the CPU/GPU
/// keep running.
```

```text
   savestate taken HERE ────────────────┐
                                        ▼
   /CS ▔▔▔╲──────────────────────────────────────────────╱▔▔▔
   MOSI    │ 03h │ AddrHi │ AddrLo │ -- │ -- │ -- │ -- │
                                     ▲
                          chip is mid-READ, 3 bytes in

   Without protocol_snapshot:  load restores an IDLE chip.
                               The game's next read gets garbage or nothing,
                               it polls forever, everything else keeps running.
                               Symptom: "the emulator is running but frozen."
```

The captured state is a plain enum
([backup.rs:81-104](core/src/hw/cartridge/backup.rs#L81-L104)):

```rust
pub enum BackupProtocolState {
    /// [`NoBackup`], or state not yet captured.
    None,
    Eeprom { mode: eeprom::Mode, write_enable: bool, value: u8 },
    Flash { mode: flash::Mode, write_enable: bool, value: u8 },
    /// [`IrBackup`]: the IR device-selector phase plus the inner [`Flash`]
    /// chip's own SPI state, flattened together so a savestate captured
    /// mid-transaction restores both.
    Ir {
        phase: ir::IrPhase,
        expecting_selector: bool,
        mode: flash::Mode,
        write_enable: bool,
        value: u8,
    },
}
```

A variant mismatch on load is _ignored_ rather than applied — loading a state
captured under a different chip type leaves the live chip alone.

### (b) The locked `.sav` file

The original implementation `mmap`ed the `.sav` for the whole session
([backup.rs:106-118](core/src/hw/cartridge/backup.rs#L106-L118)):

```rust
/// In-memory backing store for a backup chip's persistent contents.
///
/// Replaces a session-long `mmap` of the `.sav` file with a plain `Vec<u8>`
/// that is written back to disk only when the hardware itself would commit
/// (SPI chip-select release), on explicit import, or on shutdown. No file
/// handle or memory mapping is held between those points, so the file is
/// never locked against being read, replaced, or deleted by another
/// process.
pub struct SaveMem {
    path: PathBuf,
    buf: Vec<u8>,
    dirty: bool,
}
```

```text
   when does the file get written?
   ───────────────────────────────
   SPI /CS release  ──► flush()         ← the hardware's own commit point
   NDS::import_save ──► set_save_bytes() + flush
   NDS::flush_save  ──► flush()         ← call this on shutdown!
   otherwise        ──► nothing; the Vec is the truth
```

`dirty` means a clean flush is a no-op, so releasing chip-select on a read-only
transaction costs nothing.

---

## 15.6 The infrared cartridges

Pokémon HeartGold/SoulSilver, Black/White and friends put an **IR
microcontroller in front of the flash chip**. The first byte of every SPI
transaction is a device selector, not a flash opcode
([ir.rs:13-36](core/src/hw/cartridge/backup/ir.rs#L13-L36)):

```text
   normal cart                        IR cart
   ───────────                        ───────
   AUXSPI ──► flash chip              AUXSPI ──► IR MCU ──► flash chip
                                                    │
                                      first byte selects:
                                        00h → pass everything through to flash
                                        08h → IR status probe, answer AAh
                                        else → answer 00h, do not forward
```

```rust
/// On real hardware the IR MCU sits between AUXSPI and the flash chip: the
/// *first* byte of every SPI transaction selects which device the rest of
/// the transaction talks to, rather than being a flash opcode. Without this
/// wrapper, HeartGold/SoulSilver's boot-time IR status probe (opcode `08h`)
/// never receives its expected `AAh` reply, which the game interprets as a
/// broken cartridge and reports as a communication error
pub struct IrBackup {
    inner: Flash,
    phase: IrPhase,
    expecting_selector: bool,
}
```

Detection is a single character of the game code
([backup.rs:354-361](core/src/hw/cartridge/backup.rs#L354-L361)):

```rust
        let is_ir_cart =
            (header.game_code & 0xFF) as u8 == b'I' && std::env::var_os("LUNARIS_NO_IR").is_none();
```

`LUNARIS_NO_IR` exists so the wrapper can be bisected out when diagnosing a
save problem — a useful pattern for any heuristic like this.

One subtlety, documented on the `phase` field: the last response byte survives
a chip-select release, because software conventionally reads SPIDATA
_immediately after_ the write that released `/CS`. Clearing it there would
lose the reply.

---

## 15.7 Foreign save formats

Saves made by other emulators are not always raw. [`normalize_foreign_save`
in lib.rs](core/src/lib.rs#L57-L71) handles two:

```rust
pub fn normalize_foreign_save(bytes: &[u8]) -> Vec<u8> {
    if let Some(raw) = strip_desmume_footer(bytes) {
        info!(
            target: "nds_core::savedata",
            "DeSmuME .dsv footer detected; stripped to {} raw bytes",
            raw.len()
        );
        return raw.to_vec();
    }
    if bytes.starts_with(NOCASH_HEADER) {
        warn!(
            target: "nds_core::savedata",
            "no$gba save format detected; this format is not supported, treating as absent"
        );
        return Vec::new();
    }
    bytes.to_vec()
}
```

```text
   DeSmuME .dsv layout
   ┌──────────────────────────────────────────────────────┐
   │ raw save data                                        │
   ├──────────────────────────────────────────────────────┤
   │ (optional human-readable banner text)                │
   ├──────────────┬───────────────────────┬───────────────┤
   │ raw_size u32 │ 20 bytes of metadata  │ "|-DESMUME SAVE-|" │
   └──────────────┴───────────────────────┴───────────────┘
                                            └ 16-byte cookie
```

The cookie check alone is not enough — a raw save could coincidentally end with
those 16 bytes — so the declared size is sanity-checked
([lib.rs:76-90](core/src/lib.rs#L76-L90)):

```rust
    // The declared raw size must fit within the region preceding the
    // footer (which may also contain the human-readable banner text); a
    // value that doesn't is either a corrupt footer or an unlucky
    // coincidental cookie match in unrelated binary data, so bail out
    // rather than risk truncating to a bogus length.
    if raw_size > footer_start { None } else { Some(&bytes[..raw_size]) }
```

---

## 15.8 Import / export

[nds.rs:178-185](core/src/nds.rs#L178-L185):

```rust
    /// Imports external cartridge save data (e.g. from another emulator or
    /// a flashcart dump), replacing the current save and flushing it to the
    /// `.sav` file immediately. Best done at the game's title screen, since
    /// the running game may hold a stale in-RAM copy of its save data.
    pub fn import_save(&mut self, bytes: &[u8]) {
        self.hw.import_save(bytes);
    }
```

That warning is real: a game caches its save in RAM, so importing mid-session
means the game overwrites your imported data on its next write. It is a
limitation of the concept, not of the implementation.

---

## 15.9 Divergences

- **NAND saves (sram_type 8-9) are unsupported.** Those carts embed the save in
  the ROM chip and access it through _ROM commands_, not SPI at all. Lunaris
  routes them to `NoBackup` with a warning, which loses saves but never
  corrupts them. melonDS implements `CartRetailNAND` in
  `src/NDSCart/CartRetail.cpp`. Design notes:
  `docs/design/complete/fix_sav/ir-nand-foreign-sav-design.md`.
- **no$gba save format** is detected and rejected rather than converted.
- **No write timing.** Real EEPROM/Flash take milliseconds to program a page
  and report busy via the status register; Lunaris completes instantly, so a
  game's busy-wait loop simply exits on its first poll.
- **The IR MCU only answers the status probe.** Actual infrared communication
  (Pokémon's IR trading) is not emulated.

---

[← 14. The Cartridge and Boot](14_cartridge_and_boot.md) | [Next: 16. SPI: Firmware, Touchscreen, Power →](16_spi_firmware_touchscreen.md)
