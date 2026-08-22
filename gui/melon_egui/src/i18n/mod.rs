//! The UI's text, in English and Japanese.
//!
//! # How a string gets translated
//!
//! Every translatable string is a variant of [`I18nKey`]. Its **doc comment is
//! the English text** and its `#[i18n(ja = "...")]` attribute is the Japanese;
//! `i18n_derive` turns both into `const fn`s, so neither costs an allocation
//! and neither can drift out of sync with the key list. A variant with no `ja`
//! renders its English, which keeps a newly added key merely untranslated
//! rather than broken — and [`I18nKey::UNTRANSLATED`] says exactly which those
//! are, so the gap is a number rather than something a reader has to notice.
//!
//! # Overriding a translation without rebuilding
//!
//! [`I18nMap::load_with_fallback`] reads `instances/translation.<lang>.json`
//! over the built-in table, so a user who dislikes a wording can change it.
//! [`I18nMap::save_template`] writes the current language out in full as a
//! starting point. A key missing from the file keeps its built-in text, and an
//! unrecognised key deserialises to [`I18nKey::Invalid`] and is ignored — a
//! translation file from another version must never stop the emulator starting.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use snafu::ResultExt as _;

mod keys;
mod language;
mod map;

pub use keys::I18nKey;
pub use language::Language;
pub use map::{I18nMap, Translations};

#[cfg(test)]
mod tests {
    use super::{I18nKey, I18nMap, Language};

    /// Both tables must answer for every key, or a language switch would blank
    /// part of the UI.
    #[test]
    fn every_key_has_text_in_every_language() {
        for key in I18nKey::ALL {
            for language in Language::ALL {
                assert!(
                    !language.text(*key).is_empty(),
                    "{key:?} has no text in {}",
                    language.label()
                );
            }
        }
    }

    /// The point of the exercise: the Japanese must actually differ from the
    /// English. A `ja` that was forgotten falls back, and this is what catches
    /// it — stated as a count so adding a key is not an automatic failure.
    #[test]
    fn the_japanese_table_is_translated() {
        let translated =
            I18nKey::ALL.iter().filter(|key| key.default_jpn() != key.default_eng()).count();
        assert_eq!(
            I18nKey::UNTRANSLATED.len(),
            0,
            "these keys have no #[i18n(ja = ...)]: {:?}",
            I18nKey::UNTRANSLATED
        );
        assert!(
            translated * 10 >= I18nKey::ALL.len() * 9,
            "only {translated} of {} keys read differently in Japanese",
            I18nKey::ALL.len()
        );
    }

    /// Every language must be reachable through the loaded set, or a switch
    /// would silently leave the UI in the previous one.
    #[test]
    fn the_loaded_set_answers_for_every_language() {
        let translations = super::Translations::load();
        for language in Language::ALL {
            assert_eq!(translations.get(*language).language(), *language);
        }
        assert_eq!(translations.get(Language::Japanese).t(I18nKey::FileLabel), "ファイル");
        assert_eq!(translations.get(Language::English).t(I18nKey::FileLabel), "File");
    }

    #[test]
    fn a_built_in_map_translates_without_any_file() {
        let map = I18nMap::built_in(Language::Japanese);
        assert_eq!(map.t(I18nKey::FileLabel), "ファイル");
        assert_eq!(map.language(), Language::Japanese);

        let english = I18nMap::built_in(Language::English);
        assert_eq!(english.t(I18nKey::FileLabel), "File");
    }

    /// A file from another version must not stop the emulator starting, and an
    /// unknown key in it must not become visible text.
    #[test]
    fn an_unknown_key_deserialises_to_invalid() {
        let map: I18nMap =
            serde_json::from_str(r#"{"file_label": "書類", "no_such_key": "x"}"#).unwrap();
        assert_eq!(map.t(I18nKey::FileLabel), "書類");
        // Everything the file omits is simply absent from the overlay, and
        // `load_with_fallback` is what fills those in.
        assert_eq!(map.t(I18nKey::Quit), I18nKey::Quit.default_eng());
    }

    /// An override file that changes one wording must not blank the rest.
    #[test]
    fn an_overlay_only_replaces_what_it_names() {
        let mut map = I18nMap::built_in(Language::Japanese);
        let overlay: I18nMap = serde_json::from_str(r#"{"quit": "おわる"}"#).unwrap();
        for (key, text) in overlay.strings {
            if key != I18nKey::Invalid {
                map.strings.insert(key, text);
            }
        }
        assert_eq!(map.t(I18nKey::Quit), "おわる");
        assert_eq!(map.t(I18nKey::FileLabel), "ファイル");
    }
}
