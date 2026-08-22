//! Which translation the UI is drawn in.

use super::*;

/// Which translation the UI is drawn in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// The language the strings are written in, and the fallback for every key
    /// no other language has translated.
    #[default]
    English,
    Japanese,
}

impl Language {
    /// Every language, for a settings dropdown.
    pub const ALL: &'static [Self] = &[Self::English, Self::Japanese];

    /// The name of the language **in that language**, which is how a language
    /// picker has to be labelled: someone looking for Japanese is looking for
    /// 日本語, not for the word "Japanese" they may not read.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
        }
    }

    /// The suffix of this language's override file, `translation.<code>.json`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "eng",
            Self::Japanese => "jpn",
        }
    }

    /// The built-in text for `key` in this language.
    #[must_use]
    pub const fn text(self, key: I18nKey) -> &'static str {
        match self {
            Self::English => key.default_eng(),
            Self::Japanese => key.default_jpn(),
        }
    }
}
