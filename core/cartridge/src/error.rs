use std::{fmt, io, path::PathBuf};

/// Cartridge related errors.
#[derive(Debug)]
pub enum CartridgeError {
    /// Save database file could not be opened.
    OpenSaveDatabase { path: PathBuf, source: io::Error },

    /// Save database metadata could not be read.
    MetadataSaveDatabase { path: PathBuf, source: io::Error },

    /// Save database has invalid format.
    InvalidSaveDatabaseFormat { path: PathBuf, size: u64 },

    /// Save database read failed.
    ReadSaveDatabase { path: PathBuf, source: io::Error },

    /// ROM file open failed.
    OpenRom { path: PathBuf, source: io::Error },

    /// ROM metadata read failed.
    MetadataRom { path: PathBuf, source: io::Error },

    /// ROM read failed.
    ReadRom { path: PathBuf, source: io::Error },

    /// Save read failed.
    ReadSave { path: PathBuf, source: io::Error },

    /// Save write failed.
    SaveWrite { path: PathBuf, source: io::Error },
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenSaveDatabase { path, .. } => {
                write!(f, "Failed to load save database: {}", path.display())
            }
            Self::MetadataSaveDatabase { path, .. } => {
                write!(f, "Failed to read save database metadata: {}", path.display())
            }
            Self::InvalidSaveDatabaseFormat { path, size } => {
                write!(
                    f,
                    "Save database corrupted or wrong format: {} (size={size})",
                    path.display()
                )
            }
            Self::ReadSaveDatabase { path, .. } => {
                write!(f, "Failed to read save database contents: {}", path.display())
            }
            Self::OpenRom { path, .. } => write!(f, "Failed to load ROM: {}", path.display()),
            Self::MetadataRom { path, .. } => {
                write!(f, "Failed to read ROM metadata: {}", path.display())
            }
            Self::ReadRom { path, .. } => {
                write!(f, "Failed to read ROM contents: {}", path.display())
            }
            Self::ReadSave { path, .. } => {
                write!(f, "Failed to read save file: {}", path.display())
            }
            Self::SaveWrite { path, .. } => {
                write!(f, "Failed to write save file: {}", path.display())
            }
        }
    }
}

impl std::error::Error for CartridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OpenSaveDatabase { source, .. }
            | Self::MetadataSaveDatabase { source, .. }
            | Self::ReadSaveDatabase { source, .. }
            | Self::OpenRom { source, .. }
            | Self::MetadataRom { source, .. }
            | Self::ReadRom { source, .. }
            | Self::ReadSave { source, .. }
            | Self::SaveWrite { source, .. } => Some(source),
            Self::InvalidSaveDatabaseFormat { .. } => None,
        }
    }
}
