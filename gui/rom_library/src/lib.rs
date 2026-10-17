//! The start screen's game list, shared by every front end (egui, imgui,
//! melon_egui): recent ROMs plus the contents of a ROM folder, each with the
//! title and 32x32 icon from its cartridge banner.
//!
//! Only the 512-byte header and the 0x840-byte banner are read from each
//! file, so scanning a folder of 128 MiB ROMs stays instant.
//!
//! Banner layout (GBATEK "DS Cartridge Icon/Title"): the banner offset is the
//! u32 at header 0x68; inside it the icon bitmap (4x4 tiles of 8x8 pixels,
//! 4bpp, low nibble = left pixel) is at 0x20, its 16-color BGR555 palette at
//! 0x220 (index 0 transparent), and the UTF-16LE titles at 0x240, 0x100
//! bytes each in the order JP, EN, FR, DE, IT, ES.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Icon edge length in pixels.
pub const ICON_SIZE: usize = 32;

/// ROM file extensions listed in a folder.
const EXTENSIONS: [&str; 3] = ["nds", "dsi", "srl"];

/// One game on the start screen.
#[derive(Debug, Clone)]
pub struct RomEntry {
    pub path: PathBuf,
    /// File name with extension, shown as the second line.
    pub file_name: String,
    /// First line of the banner title (JP, else EN), else the file stem.
    pub title: String,
    /// 32x32 RGBA8 icon, `None` when the ROM has no banner.
    pub icon_rgba: Option<Vec<u8>>,
}

impl RomEntry {
    /// Reads the header and banner of `path`. Never fails: an unreadable or
    /// banner-less file still yields an entry titled by its file stem.
    pub fn read(path: &Path) -> Self {
        let file_name =
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let banner = read_banner(path);
        let title = banner.as_deref().and_then(banner_title).unwrap_or(stem);
        let icon_rgba = banner.as_deref().map(banner_icon);
        Self { path: path.to_path_buf(), file_name, title, icon_rgba }
    }
}

fn read_banner(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 0x6C];
    file.read_exact(&mut header).ok()?;
    let offset = u32::from_le_bytes([header[0x68], header[0x69], header[0x6A], header[0x6B]]);
    if offset < 0x200 {
        return None;
    }
    file.seek(SeekFrom::Start(u64::from(offset))).ok()?;
    let mut banner = vec![0u8; 0x840];
    file.read_exact(&mut banner).ok()?;
    Some(banner)
}

fn banner_title(banner: &[u8]) -> Option<String> {
    // JP first, then EN.
    for lang in [0usize, 1] {
        let raw = &banner[0x240 + lang * 0x100..0x240 + (lang + 1) * 0x100];
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let text = String::from_utf16_lossy(&units);
        if let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) {
            return Some(line.to_owned());
        }
    }
    None
}

fn banner_icon(banner: &[u8]) -> Vec<u8> {
    let palette: Vec<[u8; 4]> = (0..16)
        .map(|i| {
            let c = u16::from_le_bytes([banner[0x220 + i * 2], banner[0x221 + i * 2]]);
            let expand = |v: u16| ((v << 3) | (v >> 2)) as u8;
            let alpha = if i == 0 { 0 } else { 0xFF };
            [expand(c & 0x1F), expand((c >> 5) & 0x1F), expand((c >> 10) & 0x1F), alpha]
        })
        .collect();
    let mut rgba = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
    for (i, byte) in banner[0x20..0x220].iter().enumerate() {
        let tile = i / 32;
        let in_tile = i % 32;
        let x = (tile % 4) * 8 + (in_tile % 4) * 2;
        let y = (tile / 4) * 8 + in_tile / 4;
        for (dx, index) in [(0, byte & 0xF), (1, byte >> 4)] {
            let o = ((y * ICON_SIZE) + x + dx) * 4;
            rgba[o..o + 4].copy_from_slice(&palette[usize::from(index)]);
        }
    }
    rgba
}

/// ROM files directly inside `dir`, sorted by file name.
pub fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        })
        .collect();
    paths.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    paths
}

/// The start screen's list: existing `recents` first (in order), then the
/// ROMs in `rom_dir` by name, without duplicates.
pub fn scan(recents: &[PathBuf], rom_dir: Option<&Path>) -> Vec<RomEntry> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let candidates = recents
        .iter()
        .filter(|p| p.is_file())
        .cloned()
        .chain(rom_dir.map(list_dir).unwrap_or_default());
    for path in candidates {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths.iter().map(|p| RomEntry::read(p)).collect()
}

/// Puts `rom` at the front of `recents`, keeping at most `limit` entries.
pub fn push_recent(recents: &mut Vec<PathBuf>, rom: &Path, limit: usize) {
    recents.retain(|p| p != rom);
    recents.insert(0, rom.to_path_buf());
    recents.truncate(limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner_with(title: &str) -> Vec<u8> {
        let mut b = vec![0u8; 0x840];
        for (i, u) in title.encode_utf16().enumerate() {
            b[0x240 + i * 2..0x242 + i * 2].copy_from_slice(&u.to_le_bytes());
        }
        // palette[1] = pure red; first icon byte = pixel 0 -> 1, pixel 1 -> 0
        b[0x222] = 0x1F;
        b[0x20] = 0x01;
        b
    }

    #[test]
    fn title_takes_first_line_of_japanese() {
        assert_eq!(banner_title(&banner_with("ゲーム\nサブ")).as_deref(), Some("ゲーム"));
        assert_eq!(banner_title(&vec![0; 0x840]), None);
    }

    #[test]
    fn icon_decodes_tiles_and_transparency() {
        let icon = banner_icon(&banner_with("x"));
        assert_eq!(&icon[0..4], &[0xFF, 0, 0, 0xFF]);
        assert_eq!(icon[7], 0, "index 0 is transparent");
    }

    #[test]
    fn missing_file_falls_back_to_stem() {
        let e = RomEntry::read(Path::new("no/such/Game Name.nds"));
        assert_eq!(e.title, "Game Name");
        assert!(e.icon_rgba.is_none());
    }

    /// `LUNARIS_ROM_DIR=<dir> cargo test -p lunaris_rom_library -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a real ROM folder"]
    fn scan_real_folder() {
        let dir = std::env::var("LUNARIS_ROM_DIR").unwrap_or_default();
        for e in scan(&[], Some(Path::new(&dir))) {
            println!("{} | {} | icon={}", e.title, e.file_name, e.icon_rgba.is_some());
        }
    }

    #[test]
    fn recents_dedup_and_cap() {
        let mut r = vec![PathBuf::from("a"), PathBuf::from("b")];
        push_recent(&mut r, Path::new("b"), 2);
        assert_eq!(r, [PathBuf::from("b"), PathBuf::from("a")]);
    }
}

#[cfg(feature = "egui")]
pub mod egui_launcher;
