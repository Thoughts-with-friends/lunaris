//! A font that can draw Japanese, Chinese and Korean.
//!
//! egui ships one proportional and one monospace font, both Latin-only, so
//! every CJK character in a ROM's filename, a cheat's name or a status line
//! comes out as `□`. Nothing is bundled — a CJK face is megabytes — so the
//! system's own font is borrowed.
//!
//! It is added as a *fallback*, at the end of each family, rather than put in
//! front: egui takes each character from the first font in the family that has
//! it, so Latin text keeps egui's own face and only the characters it lacks
//! come from the system font.

use std::path::{Path, PathBuf};

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// Where to look, best first. A collection (`.ttc`) is read at index 0, which
/// is the regular weight in every one of these.
///
/// Japanese faces come first deliberately: Chinese and Korean fonts also cover
/// kana and kanji, but with the shapes of their own language, and a Japanese
/// ROM title in Chinese glyph forms looks wrong to the person reading it.
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
const OVERRIDE: &str = "LUNARIS_FONT";

/// Installs a CJK fallback, returning the file it came from.
///
/// `font_path` is the caller's own preference, tried ahead of everything else;
/// `None` leaves the choice to [`OVERRIDE`] and the candidate list. A `None`
/// return means nothing could be read, which is not fatal: the UI works, it
/// just cannot draw those characters.
pub(crate) fn setup_custom_fonts<A>(ctx: &Context, font_path: Option<A>) -> Option<PathBuf>
where
    A: AsRef<Path>,
{
    let (path, bytes) = load(font_path.as_ref().map(AsRef::as_ref))?;

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("cjk".to_owned(), FontData::from_owned(bytes).into());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    nds_core::log::info!("fonts: CJK fallback from {}", path.display());
    Some(path)
}

/// The caller's font, the [`OVERRIDE`], or the first candidate that reads.
fn load(preferred: Option<&Path>) -> Option<(PathBuf, Vec<u8>)> {
    let explicit =
        preferred.map(Path::to_path_buf).or_else(|| std::env::var_os(OVERRIDE).map(PathBuf::from));
    if let Some(path) = explicit {
        match std::fs::read(&path) {
            Ok(bytes) => return Some((path, bytes)),
            Err(e) => {
                nds_core::log::error!("fonts: cannot read {}: {e}", path.display());
                // Fall through to the candidates rather than leaving the UI
                // unable to draw anything but Latin.
            }
        }
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
    fn whatever_is_found_is_big_enough_to_be_a_font() {
        // Not every host has one — a bare container has no fonts at all — so
        // this asserts about the result rather than that there is one.
        if let Some((path, bytes)) = load(None) {
            assert!(bytes.len() > 1024, "{} is too small to be a font", path.display());
        }
    }
}
