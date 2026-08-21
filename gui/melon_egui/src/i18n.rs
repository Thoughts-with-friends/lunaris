use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use snafu::ResultExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize, i18n_derive::I18n)]
#[serde(rename_all = "snake_case")]
pub enum I18nKey {
    /// File
    FileLabel,

    // NOTE: Using `skip_serializing` causes an error when attempting to serialize `Invalid`.
    /// Invalid key comes here when deserializing unknown strings.
    #[serde(other)]
    Invalid,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct I18nMap(IndexMap<I18nKey, Cow<'static, str>>);

impl I18nMap {
    pub fn new() -> Self {
        Self(IndexMap::default())
    }

    /// Translate given key or fallback to default English.
    pub fn t(&self, key: I18nKey) -> &str {
        self.0.get(&key).map_or_else(|| key.default_eng(), |s| s.as_ref())
    }

    /// Try to load path & parse i18n map.
    ///
    /// # Errors
    /// failed to read json
    #[inline]
    pub fn load<P>(path: P) -> Result<Self, Error>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();

        let content = std::fs::read_to_string(path).with_context(|_| ReadFileSnafu { path })?;
        let map: Self = serde_json::from_str(&content).with_context(|_| ParseJsonSnafu { path })?;
        Ok(map)
    }

    /// Try to load `./translation.json`.
    /// If not exists or failed to parse, fallback to `default_map()`.
    ///
    /// # Errors
    /// failed to read json
    pub fn load_with_fallback() -> Result<Self, Error> {
        let path = Self::i18n_path();
        let path = path.as_path();

        if !path.exists() {
            log::info!("{} does not exist.", path.display());
            return Ok(Self::new());
        }

        Self::load(path)
    }

    /// Return the path used for the shared translation map.
    pub fn i18n_path() -> PathBuf {
        crate::config::instances_dir().join("translation.json")
    }

    /// Save translation.json
    /// # Errors
    /// failed to write json
    pub fn save() -> Result<(), Error> {
        let path = Self::i18n_path();
        let path = path.as_path();

        let map =
            Self(I18nKey::ALL.iter().map(|key| (*key, Cow::Borrowed(key.default_eng()))).collect());

        let text = serde_json::to_string_pretty(&map).context(SerializeJsonSnafu { path })?;
        std::fs::write(path, text).context(WriteFileSnafu { path })?;
        Ok(())
    }
}

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("Failed to read file: {}", path.display()))]
    ReadFile { path: PathBuf, source: std::io::Error },

    #[snafu(display("Failed to write file: {}", path.display()))]
    WriteFile { path: PathBuf, source: std::io::Error },

    #[snafu(display("Failed to parse json: {}", path.display()))]
    ParseJson { path: PathBuf, source: serde_json::Error },

    #[snafu(display("Failed to serialize json: {}", path.display()))]
    SerializeJson { path: PathBuf, source: serde_json::Error },
}
