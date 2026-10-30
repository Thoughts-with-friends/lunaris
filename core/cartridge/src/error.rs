use std::path::PathBuf;

/// Cartridge related errors.
#[derive(Debug, snafu::Snafu)]
pub enum CartridgeError {
    /// Save database file could not be opened.
    #[snafu(display("Failed to load save database: {}", path.display()))]
    OpenSaveDatabase { path: PathBuf, source: std::io::Error },

    /// Save database metadata could not be read.
    #[snafu(display("Failed to read save database metadata: {}", path.display()))]
    MetadataSaveDatabase { path: PathBuf, source: std::io::Error },

    /// Save database has invalid format.
    #[snafu(display("Save database corrupted or wrong format: {} (size={size})", path.display()))]
    InvalidSaveDatabaseFormat { path: PathBuf, size: u64 },

    /// Save database read failed.
    #[snafu(display("Failed to read save database contents: {}", path.display()))]
    ReadSaveDatabase { path: PathBuf, source: std::io::Error },

    /// ROM file open failed.
    #[snafu(display("Failed to load ROM: {}", path.display()))]
    OpenRom { path: PathBuf, source: std::io::Error },

    /// ROM metadata read failed.
    #[snafu(display("Failed to read ROM metadata: {}", path.display()))]
    MetadataRom { path: PathBuf, source: std::io::Error },

    /// ROM read failed.
    #[snafu(display("Failed to read ROM contents: {}", path.display()))]
    ReadRom { path: PathBuf, source: std::io::Error },

    /// Save read failed.
    #[snafu(display("Failed to read save file: {}", path.display()))]
    ReadSave { path: PathBuf, source: std::io::Error },

    #[snafu(display("Failed to write save file: {}", path.display()))]
    SaveWrite { path: PathBuf, source: std::io::Error },
}
