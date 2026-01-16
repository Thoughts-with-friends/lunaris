use std::path::PathBuf;

#[derive(Debug, snafu::Snafu)]
#[snafu(visibility(pub))]
pub enum EmuError {
    /// Failed to write file.
    #[snafu(display("Failed to write file: {}", path.display()))]
    FailedWriteFile {
        source: std::io::Error,
        path: PathBuf,
    },

    /// Failed to read file.
    #[snafu(display("Failed to read file: {}", path.display()))]
    FailedReadFile {
        source: std::io::Error,
        path: PathBuf,
    },
}
