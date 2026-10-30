use std::{fmt, io, path::PathBuf};

#[derive(Debug)]
pub enum EmuError {
    /// Failed to write file.
    FailedWriteFile { source: io::Error, path: PathBuf },

    /// Failed to read file.
    FailedReadFile { source: io::Error, path: PathBuf },
}

impl EmuError {
    /// A [`Self::FailedReadFile`] for `path`, for `map_err`.
    pub fn read(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::FailedReadFile { source, path }
    }
}

impl fmt::Display for EmuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FailedWriteFile { path, .. } => {
                write!(f, "Failed to write file: {}", path.display())
            }
            Self::FailedReadFile { path, .. } => {
                write!(f, "Failed to read file: {}", path.display())
            }
        }
    }
}

impl std::error::Error for EmuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FailedWriteFile { source, .. } | Self::FailedReadFile { source, .. } => {
                Some(source)
            }
        }
    }
}
