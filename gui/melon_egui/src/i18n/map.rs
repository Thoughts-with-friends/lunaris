//! The strings themselves: the built-in tables, and the files that override
//! them.

use super::*;

/// The text the UI draws, for one language.
///
/// Built from the language's built-in table and then overlaid with whatever
/// `instances/translation.<code>.json` holds, so the map is always complete:
/// [`I18nMap::t`] never has to fall back at draw time.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct I18nMap {
    /// Which language this map is, so a settings pane can show it and
    /// [`I18nMap::save_template`] knows what to write.
    #[serde(default)]
    pub(crate) language: Language,
    #[serde(flatten)]
    pub(crate) strings: IndexMap<I18nKey, Cow<'static, str>>,
}

impl Default for I18nMap {
    /// English, which is what the front end draws before any setting has been
    /// read.
    fn default() -> Self {
        Self::built_in(Language::English)
    }
}

impl I18nMap {
    /// The complete built-in table for `language`, with no file overlaid.
    #[must_use]
    pub fn built_in(language: Language) -> Self {
        let strings =
            I18nKey::ALL.iter().map(|key| (*key, Cow::Borrowed(language.text(*key)))).collect();
        Self { language, strings }
    }

    /// Which language this map holds.
    #[must_use]
    pub const fn language(&self) -> Language {
        self.language
    }

    /// Translate `key`, falling back to the built-in English for a key the map
    /// somehow lacks.
    #[must_use]
    pub fn t(&self, key: I18nKey) -> &str {
        self.strings.get(&key).map_or_else(|| key.default_eng(), AsRef::as_ref)
    }

    /// Translate `key` into an owned string, for the many call sites that
    /// build a label with [`format!`] and cannot hold a borrow of `self` while
    /// they also borrow it mutably.
    #[must_use]
    pub fn s(&self, key: I18nKey) -> String {
        self.t(key).to_owned()
    }

    /// Load `path` as a translation file.
    ///
    /// # Errors
    /// If the file cannot be read or is not the JSON object this writes.
    #[inline]
    pub fn load<P>(path: P) -> Result<Self, Error>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).with_context(|_| ReadFileSnafu { path })?;
        serde_json::from_str(&content).with_context(|_| ParseJsonSnafu { path })
    }

    /// The table for `language`, with its override file applied if there is one.
    ///
    /// A missing file is not an error: it is the ordinary case, and the
    /// built-in table is already complete. A file that is present but
    /// unreadable *is* reported, because a user who wrote one wants to know it
    /// was ignored.
    ///
    /// # Errors
    /// If the override file exists but cannot be read or parsed.
    pub fn load_with_fallback(language: Language) -> Result<Self, Error> {
        let mut map = Self::built_in(language);
        let path = Self::i18n_path(language);
        if !path.exists() {
            log::info!("{} does not exist; using the built-in text.", path.display());
            return Ok(map);
        }
        let overlay = Self::load(&path)?;
        // Overlaid key by key rather than replacing the map: a file that only
        // changes one wording must not blank out everything it omits.
        for (key, text) in overlay.strings {
            if key != I18nKey::Invalid {
                map.strings.insert(key, text);
            }
        }
        Ok(map)
    }

    /// Where `language`'s override file lives.
    #[must_use]
    pub fn i18n_path(language: Language) -> PathBuf {
        // Shared between instances rather than per-instance: a translation is a
        // property of the person reading the screen, not of one console.
        PathBuf::from(crate::file::settings::INSTANCES_DIR)
            .join(format!("translation.{}.json", language.code()))
    }

    /// Write this map out as a starting point for a hand translation.
    ///
    /// # Errors
    /// If the directory cannot be made, or the file cannot be written.
    pub fn save_template(&self) -> Result<PathBuf, Error> {
        let path = Self::i18n_path(self.language);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|_| WriteFileSnafu { path: path.clone() })?;
        }
        let text = serde_json::to_string_pretty(self)
            .with_context(|_| SerializeJsonSnafu { path: path.clone() })?;
        std::fs::write(&path, text).with_context(|_| WriteFileSnafu { path: path.clone() })?;
        Ok(path)
    }
}

/// Every language's strings, read once.
///
/// # Why they are all loaded up front
///
/// The two consoles have settings of their own, and may be set to different
/// languages. The front end applies each console's settings as it draws that
/// console's window, so a language switch happens *twice per repaint* whenever
/// the second window is open. Reading a file and rebuilding a map that often
/// would be absurd; reading both files once and indexing is not.
#[derive(Debug)]
pub struct Translations([I18nMap; Language::ALL.len()]);

impl Default for Translations {
    fn default() -> Self {
        Self::load()
    }
}

impl Translations {
    /// Read every language, applying each one's override file if it has one.
    ///
    /// A file that cannot be read is reported and skipped, leaving that
    /// language on its built-in text — a broken translation must never stop the
    /// emulator starting.
    #[must_use]
    pub fn load() -> Self {
        Self(std::array::from_fn(|index| {
            let language = Language::ALL[index];
            I18nMap::load_with_fallback(language).unwrap_or_else(|error| {
                log::warn!("{error}; using the built-in text");
                I18nMap::built_in(language)
            })
        }))
    }

    /// The strings for one language.
    #[must_use]
    pub fn get(&self, language: Language) -> &I18nMap {
        // `Language::ALL` is in declaration order, so the discriminant is the
        // index; the search keeps that from being a silent assumption.
        let index = Language::ALL.iter().position(|it| *it == language).unwrap_or(0);
        let map = &self.0[index];
        debug_assert_eq!(map.language(), language, "Translations is indexed out of order");
        map
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
