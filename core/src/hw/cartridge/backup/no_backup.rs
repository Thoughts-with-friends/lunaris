use super::Backup;

pub struct NoBackup {}

impl Backup for NoBackup {
    fn read(&self) -> u8 {
        0
    }
    fn write(&mut self, _hold: bool, _value: u8) {}

    fn protocol_snapshot(&self) -> super::BackupProtocolState {
        super::BackupProtocolState::None
    }

    fn restore_protocol_state(&mut self, _state: super::BackupProtocolState) {}

    fn save_bytes(&self) -> Option<&[u8]> {
        None
    }

    fn set_save_bytes(&mut self, _bytes: &[u8]) {}

    fn flush(&mut self) {}
}

impl NoBackup {
    pub fn new() -> Self {
        NoBackup {}
    }
}
