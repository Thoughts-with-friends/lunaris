//! Save files written by other emulators.
//!
//! A DeSmuME `.dsv` is the raw chip image plus a binary footer; a no$gba
//! save is a container that is not parsed (only detected). melonDS saves
//! are already raw.

/// DeSmuME's footer (`BackupDeviceFileInfo` + `BackupDeviceFileSaveFooter`,
/// little-endian): `size, padSize, type, addr_size, mem_size, version` (u32
/// each), then this 16-byte cookie. `size` is the unpadded payload length.
const DESMUME_FOOTER_COOKIE: &[u8; 16] = b"|-DESMUME SAVE-|";
const DESMUME_FOOTER_LEN: usize = 4 * 6 + 16;

/// no$gba backup files start with this; their body is not a raw image.
const NOCASH_HEADER: &[u8] = b"NocashGbaBackupMediaSavDataFile";

/// The raw chip image inside `bytes`: a DeSmuME footer is stripped, a
/// no$gba container gives an empty image (unsupported), anything else is
/// returned as is.
pub fn normalize_foreign_save(bytes: &[u8]) -> Vec<u8> {
    if let Some(raw) = strip_desmume_footer(bytes) {
        #[cfg(feature = "log")]
        log::info!("DeSmuME .dsv footer detected; stripped to {} raw bytes", raw.len());
        return raw.to_vec();
    }
    if bytes.starts_with(NOCASH_HEADER) {
        #[cfg(feature = "log")]
        log::warn!("no$gba save format detected; it is not supported, treating as absent");
        return Vec::new();
    }
    bytes.to_vec()
}

fn strip_desmume_footer(bytes: &[u8]) -> Option<&[u8]> {
    let footer_start = bytes.len().checked_sub(DESMUME_FOOTER_LEN)?;
    if &bytes[bytes.len() - 16..] != DESMUME_FOOTER_COOKIE {
        return None;
    }
    let size = bytes.get(footer_start..footer_start + 4)?;
    let raw_size = u32::from_le_bytes([size[0], size[1], size[2], size[3]]) as usize;
    // A size past the footer is a corrupt footer or a chance cookie match.
    (raw_size <= footer_start).then(|| &bytes[..raw_size])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_desmume_footer() {
        let mut dsv = vec![0xAB; 8];
        dsv.extend_from_slice(b"banner");
        dsv.extend_from_slice(&8_u32.to_le_bytes());
        dsv.extend_from_slice(&[0; 20]);
        dsv.extend_from_slice(DESMUME_FOOTER_COOKIE);
        assert_eq!(normalize_foreign_save(&dsv), vec![0xAB; 8]);
    }

    #[test]
    fn raw_and_nocash_saves() {
        assert_eq!(normalize_foreign_save(&[1, 2, 3]), vec![1, 2, 3]);
        assert!(normalize_foreign_save(b"NocashGbaBackupMediaSavDataFile....").is_empty());
    }
}
