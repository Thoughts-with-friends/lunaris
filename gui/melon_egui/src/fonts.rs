//! A font that can draw Japanese, Chinese and Korean.
//!
//! egui ships one proportional font and one monospace font, both Latin-only:
//! every CJK character in a ROM's filename, a cheat's name or an OSD message
//! comes out as `□`. Nothing here is bundled — a CJK face is megabytes and this
//! front end is deliberately one self-contained executable — so the system's
//! own font is borrowed instead.
//!
//! It is added as a *fallback*, at the end of each family: egui takes each
//! character from the first font in the family that has it, so Latin text keeps
//! egui's own face and only the characters it lacks come from the system font.

use std::path::{Path, PathBuf};

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// Where to look, best first. A collection (`.ttc`) is read at index 0, which
/// is the regular weight in every one of these.
///
/// The Japanese faces come first deliberately: several Chinese and Korean
/// fonts also cover kana and kanji, but with the shapes of their own language,
/// and a Japanese ROM title rendered in Chinese glyph forms looks wrong to the
/// person reading it.
const CANDIDATES: &[&str] = &[
    // Windows.
    r"C:\Windows\Fonts\meiryo.ttc",
    r"C:\Windows\Fonts\YuGothR.ttc",
    r"C:\Windows\Fonts\YuGothM.ttc",
    r"C:\Windows\Fonts\msgothic.ttc",
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\malgun.ttf",
    // macOS.
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    // Linux, in the usual packaging locations.
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJKjp-Regular.otf",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/vlgothic/VL-Gothic-Regular.ttf",
    "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
];

/// The environment variable that overrides the search: point it at a `.ttf`,
/// `.otf` or `.ttc` to use that instead.
const OVERRIDE: &str = "MELON_EGUI_FONT";

/// Install a CJK fallback, returning the file it came from.
///
/// `None` means none of the candidates could be read, which is not fatal — the
/// UI works, it just cannot draw those characters.
pub fn install(ctx: &Context) -> Option<PathBuf> {
    let (path, bytes) = load()?;

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "cjk".to_owned(),
        // Index 0: the first face of a collection. The alternatives in a `.ttc`
        // are other weights, and picking a weight per family is more machinery
        // than a fallback deserves.
        std::sync::Arc::new(FontData::from_owned(bytes)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    Some(path)
}

/// The first candidate that can be read, or the [`OVERRIDE`] if it is set.
fn load() -> Option<(PathBuf, Vec<u8>)> {
    if let Some(path) = std::env::var_os(OVERRIDE) {
        let path = PathBuf::from(path);
        return match std::fs::read(&path) {
            Ok(bytes) => Some((path, bytes)),
            Err(e) => {
                log::warn!("{OVERRIDE}={} could not be read: {e}", path.display());
                None
            }
        };
    }
    CANDIDATES
        .iter()
        .map(Path::new)
        .find_map(|path| std::fs::read(path).ok().map(|bytes| (path.to_path_buf(), bytes)))
}

#[cfg(test)]
mod tests {
    use super::{CANDIDATES, load};

    #[test]
    fn the_candidates_are_absolute_paths() {
        // A relative path would resolve against the working directory, which
        // for a windowed program is wherever it happened to be launched from.
        // Checked by shape rather than with `Path::is_absolute`, which answers
        // for the *host* platform: a Unix path is not "absolute" on Windows,
        // and the list deliberately carries both.
        for path in CANDIDATES {
            let unix = path.starts_with('/');
            let windows = path.as_bytes().get(1) == Some(&b':');
            assert!(unix || windows, "{path} is not an absolute path");
        }
    }

    #[test]
    fn a_font_is_found_on_this_machine() {
        // Not a guarantee for every host -- a bare container has no fonts at
        // all -- so this only asserts that what `load` returns is usable.
        if let Some((path, bytes)) = load() {
            assert!(bytes.len() > 1024, "{} is too small to be a font", path.display());
        }
    }
}
