//! Infrared MCU wrapper for retail carts whose backup flash is accessed
//! through an intermediary IR chip (Pokémon HeartGold/SoulSilver, Black/
//! White, and other game codes starting with ASCII `I`).
//!
//! GBATEK "DS Cartridge Backup": <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
//! melonDS `CartRetailIR` (`src/NDSCart/CartRetail.cpp`) is the reference
//! implementation this module mirrors.
//!
//! See `docs/design/ir-nand-foreign-sav-design.md` §2.1/§3.1.

use super::{Backup, BackupProtocolState, Flash};

/// Wraps a [`Flash`] chip behind the IR MCU's device-selector protocol.
///
/// On real hardware the IR MCU sits between AUXSPI and the flash chip: the
/// *first* byte of every SPI transaction selects which device the rest of
/// the transaction talks to, rather than being a flash opcode. Without this
/// wrapper, HeartGold/SoulSilver's boot-time IR status probe (opcode `08h`)
/// never receives its expected `AAh` reply, which the game interprets as a
/// broken cartridge and reports as a communication error — see
/// `docs/design/ir-nand-foreign-sav-design.md` §2.1.
pub struct IrBackup {
    inner: Flash,
    /// What the *next* [`Backup::read`] call should return. Deliberately
    /// left untouched by chip-select release (unlike `expecting_selector`
    /// below), mirroring how [`Flash`]/EEPROM keep their last response byte
    /// live after `/CS` deasserts: software conventionally reads SPIDATA
    /// immediately after the write that produced it, which may be the same
    /// call that also released `/CS`.
    phase: IrPhase,
    /// `true` at the start of a transaction (after construction, or right
    /// after a `/CS` release): the *next* byte written is a device
    /// selector rather than being routed by `phase`.
    expecting_selector: bool,
}

impl IrBackup {
    pub fn new(inner: Flash) -> Self {
        IrBackup { inner, phase: IrPhase::AwaitingCommand, expecting_selector: true }
    }
}

impl Backup for IrBackup {
    fn read(&self) -> u8 {
        match self.phase {
            IrPhase::AwaitingCommand => 0,
            IrPhase::PassThrough => self.inner.read(),
            IrPhase::IrReply(reply) => reply,
        }
    }

    fn write(&mut self, hold: bool, value: u8) {
        if self.expecting_selector {
            // The device-selector byte itself: 00h hands the rest of the
            // transaction to the flash chip untouched; 08h is a status
            // probe answered with a fixed AAh; anything else gets a safe
            // 00h reply rather than being forwarded, since the IR MCU's
            // other operations (send/receive IR data) are not emulated.
            self.phase = match value {
                0x00 => IrPhase::PassThrough,
                0x08 => IrPhase::IrReply(0xAA),
                _ => IrPhase::IrReply(0x00),
            };
            self.expecting_selector = false;
        } else {
            match self.phase {
                // Forwarding also lets the inner Flash observe its own
                // chip-select release below (write-enable clear + SaveMem
                // flush), since `hold` is passed through unchanged.
                IrPhase::PassThrough => self.inner.write(hold, value),
                IrPhase::IrReply(_) => {}
                IrPhase::AwaitingCommand => unreachable!(
                    "phase is only AwaitingCommand before the first selector byte, \
                     while expecting_selector is still true"
                ),
            }
        }
        if !hold {
            // Only the "expect a selector next" flag resets here; `phase`
            // is left alone so the response to this very byte remains
            // readable afterwards (see `phase`'s doc comment).
            self.expecting_selector = true;
        }
    }

    /// Combines the IR selector state with the inner [`Flash`] chip's own
    /// snapshot, so a savestate captured mid-transaction (e.g. right after
    /// the `08h` probe byte, before the reply has been read) restores
    /// correctly instead of leaving the game waiting on a reply that never
    /// comes. See [`BackupProtocolState::Ir`].
    fn protocol_snapshot(&self) -> BackupProtocolState {
        match self.inner.protocol_snapshot() {
            BackupProtocolState::Flash { mode, write_enable, value } => BackupProtocolState::Ir {
                phase: self.phase,
                expecting_selector: self.expecting_selector,
                mode,
                write_enable,
                value,
            },
            _ => unreachable!("Flash::protocol_snapshot always returns BackupProtocolState::Flash"),
        }
    }

    fn restore_protocol_state(&mut self, state: BackupProtocolState) {
        if let BackupProtocolState::Ir { phase, expecting_selector, mode, write_enable, value } =
            state
        {
            self.phase = phase;
            self.expecting_selector = expecting_selector;
            self.inner.restore_protocol_state(BackupProtocolState::Flash {
                mode,
                write_enable,
                value,
            });
        }
    }

    fn save_bytes(&self) -> Option<&[u8]> {
        self.inner.save_bytes()
    }

    fn set_save_bytes(&mut self, bytes: &[u8]) {
        self.inner.set_save_bytes(bytes);
    }

    fn flush(&mut self) {
        self.inner.flush();
    }
}

/// IR device-selector state, driven by the first byte of each SPI
/// transaction. See [`IrBackup`].
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum IrPhase {
    AwaitingCommand,
    PassThrough,
    IrReply(u8),
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_flash(name: &str) -> Flash {
        let path =
            std::env::temp_dir().join(format!("lunaris_ir_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_file(&path);
        Flash::new_backup(path, 0x8_0000)
    }

    /// A `08h` selector byte followed by any number of clocked bytes must
    /// yield `AAh` on every read, matching melonDS `CartRetailIR`'s status
    /// probe reply -- including immediately after the byte that released
    /// `/CS`, since software conventionally reads SPIDATA right after the
    /// write that produced the response. This is the exact exchange
    /// HeartGold/SoulSilver perform at boot; failing it produces the
    /// in-game "communication error" screen (see module docs).
    #[test]
    fn status_probe_replies_aa_repeatedly() {
        let mut ir = IrBackup::new(temp_flash("probe"));
        ir.write(true, 0x08);
        assert_eq!(ir.read(), 0xAA);
        ir.write(true, 0x00);
        assert_eq!(ir.read(), 0xAA);
        ir.write(false, 0x00);
        assert_eq!(ir.read(), 0xAA);

        // A fresh transaction after deselect must probe again correctly.
        ir.write(true, 0x08);
        assert_eq!(ir.read(), 0xAA);
    }

    /// A `00h` selector byte forwards the rest of the transaction to the
    /// flash chip unchanged: a WREN + Page Write + read-back round-trip
    /// through the IR wrapper must match writing directly to an unwrapped
    /// `Flash` with the same starting contents.
    #[test]
    fn zero_selector_passes_through_to_flash() {
        let mut ir = IrBackup::new(temp_flash("passthrough"));

        // WREN
        ir.write(true, 0x00); // IR selector: pass-through
        ir.write(false, 0x06); // WREN, released immediately

        // Page Write at address 0x10: 0x0A, 3 address bytes, 1 data byte
        ir.write(true, 0x00); // IR selector
        ir.write(true, 0x0A);
        ir.write(true, 0x00);
        ir.write(true, 0x00);
        ir.write(true, 0x10);
        ir.write(false, 0x77);

        // Read back at address 0x10: 0x03, 3 address bytes, 1 dummy read byte
        ir.write(true, 0x00); // IR selector
        ir.write(true, 0x03);
        ir.write(true, 0x00);
        ir.write(true, 0x00);
        ir.write(true, 0x10);
        ir.write(false, 0x00);

        assert_eq!(ir.read(), 0x77);
    }

    /// Chip-select release outside of an active pass-through transaction
    /// (e.g. after just an unrecognized selector byte) must correctly
    /// start a fresh transaction on the next write, rather than getting
    /// stuck interpreting further bytes as a continuation.
    #[test]
    fn deselect_resets_to_awaiting_command() {
        let mut ir = IrBackup::new(temp_flash("deselect"));
        ir.write(true, 0x2A); // unrecognized selector -> IrReply(0x00)
        assert_eq!(ir.read(), 0x00);
        ir.write(false, 0x00);

        // A fresh 08h probe after deselect must behave normally.
        ir.write(true, 0x08);
        assert_eq!(ir.read(), 0xAA);
    }

    /// A savestate captured mid-pass-through (after WREN, mid Page-Write
    /// address bytes) must restore both the IR selector state and the
    /// inner flash's SPI state, so the transaction can complete correctly
    /// after load instead of hanging the game waiting on a reply.
    #[test]
    fn savestate_round_trip_mid_transaction() {
        let mut ir = IrBackup::new(temp_flash("savestate"));

        // WREN, in its own transaction.
        ir.write(true, 0x00); // IR selector: pass-through
        ir.write(false, 0x06);

        ir.write(true, 0x00); // IR selector: pass-through
        ir.write(true, 0x0A); // Page Write opcode
        ir.write(true, 0x00); // address byte 1/3

        let snapshot = ir.protocol_snapshot();

        let mut restored = IrBackup::new(temp_flash("savestate_restored"));
        restored.restore_protocol_state(snapshot);

        // Finish the transaction on the restored instance: 2 more address
        // bytes then the data byte.
        restored.write(true, 0x00);
        restored.write(true, 0x20);
        restored.write(false, 0x55);

        // Read back through a fresh pass-through transaction.
        restored.write(true, 0x00);
        restored.write(true, 0x03);
        restored.write(true, 0x00);
        restored.write(true, 0x00);
        restored.write(true, 0x20);
        restored.write(false, 0x00);
        assert_eq!(restored.read(), 0x55);
    }
}
