use std::fmt::{self, Write};

use nds_core::{ArCode, CheatMap};

/// Parses a sequence of Action Replay style cheat codes.
///
/// Each code is a run of `addr value` hex-pair lines. A blank line, or a
/// comment-only line (e.g. `// Money Max`), closes the code in progress and
/// starts a new one. A comment-only line whose text is exactly `disabled`
/// (case-insensitive) marks the following code as disabled.
///
/// ```txt
/// // Money Max
/// 94000130 FFFB0000
/// 00000088 000F423F
/// D2000000 00000000
///
/// // disabled
/// // Catch Rate 100%
/// 92246B5A 00002801
/// 12246B5A 00004280
/// D2000000 00000000
/// ```
pub fn cheat_map_from_str(text: &str) -> Result<CheatMap, CheatMapParseError> {
    let mut cheats: CheatMap = Vec::new();
    let mut current: Vec<u32> = Vec::new();
    let mut current_enabled = true;

    for (line_no, raw_line) in text.lines().enumerate() {
        let (code_part, comment) = match raw_line.split_once("//") {
            Some((code, comment)) => (code, Some(comment)),
            None => (raw_line, None),
        };
        let code_part = code_part.trim();

        if code_part.is_empty() {
            // Blank line or comment-only line: close off the in-progress
            // code (if any) and configure the one that follows.
            if !current.is_empty() {
                cheats
                    .push(ArCode { code: std::mem::take(&mut current), enabled: current_enabled });
            }

            current_enabled =
                !comment.map(|c| c.trim().eq_ignore_ascii_case("disabled")).unwrap_or(false);

            continue;
        }

        // e.g. 0223_DD38 309C_1C28
        let mut parts = code_part.split_whitespace();
        let addr = parts.next().ok_or(CheatMapParseError::InvalidFormat(line_no + 1))?;
        let value = parts.next().ok_or(CheatMapParseError::InvalidFormat(line_no + 1))?;

        if parts.next().is_some() {
            return Err(CheatMapParseError::InvalidFormat(line_no + 1));
        }

        let addr = parse_hex_u32(addr)
            .map_err(|_| CheatMapParseError::InvalidHex(line_no + 1, addr.into()))?;
        let value = parse_hex_u32(value)
            .map_err(|_| CheatMapParseError::InvalidHex(line_no + 1, value.into()))?;

        current.push(addr);
        current.push(value);
    }

    if !current.is_empty() {
        cheats.push(ArCode { code: current, enabled: current_enabled });
    }

    Ok(cheats)
}

pub fn cheat_map_to_string(cheat_map: &CheatMap) -> String {
    let mut out = String::new();

    for (i, arcode) in cheat_map.iter().enumerate() {
        if i > 0 {
            let _ = writeln!(out);
        }

        if !arcode.enabled {
            let _ = writeln!(out, "// disabled");
        }

        for pair in arcode.code.as_chunks::<2>().0 {
            let (addr, value) = (pair[0], pair[1]);
            let _ = writeln!(
                out,
                "{:04X}_{:04X} {:04X}_{:04X}",
                addr >> 16,
                addr & 0xFFFF,
                value >> 16,
                value & 0xFFFF
            );
        }
    }

    out
}

fn parse_hex_u32(s: &str) -> Result<u32, std::num::ParseIntError> {
    u32::from_str_radix(&s.replace('_', ""), 16)
}

#[derive(Debug)]
pub enum CheatMapParseError {
    InvalidFormat(usize),
    InvalidHex(usize, String),
}

impl fmt::Display for CheatMapParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(line) => {
                write!(f, "invalid format at line {}", line)
            }
            Self::InvalidHex(line, value) => {
                write!(f, "invalid hex '{}' at line {}", value, line)
            }
        }
    }
}

impl std::error::Error for CheatMapParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_code() {
        let text = r#"
0223_DD34 6008_0180
0223_DD38 309C_1C28
"#;

        let cheats = cheat_map_from_str(text).unwrap();

        assert_eq!(cheats.len(), 1);
        assert_eq!(cheats[0].code, vec![0x0223_DD34, 0x6008_0180, 0x0223_DD38, 0x309C_1C28]);
        assert!(cheats[0].enabled);
    }

    #[test]
    fn parse_multiple_codes_separated_by_comments() {
        // Mirrors real-world dumps where each code is introduced by a name
        // comment, with no blank lines in between.
        let text = r#"
// Money Max
94000130 FFFB0000
D2000000 00000000
// Catch Rate 100%
92246B5A 00002801
D2000000 00000000
"#;

        let cheats = cheat_map_from_str(text).unwrap();

        assert_eq!(cheats.len(), 2);
        assert_eq!(cheats[0].code, vec![0x9400_0130, 0xFFFB_0000, 0xD200_0000, 0x0000_0000]);
        assert_eq!(cheats[1].code, vec![0x9224_6B5A, 0x0000_2801, 0xD200_0000, 0x0000_0000]);

        // The repeated `D2000000` terminator must survive in both codes,
        // unlike the old IndexMap-based model where it would collide.
        assert!(cheats[0].code.contains(&0xD200_0000));
        assert!(cheats[1].code.contains(&0xD200_0000));
    }

    #[test]
    fn parse_disabled_marker() {
        let text = r#"
// disabled
94000130 FFFB0000
D2000000 00000000
"#;

        let cheats = cheat_map_from_str(text).unwrap();

        assert_eq!(cheats.len(), 1);
        assert!(!cheats[0].enabled);
    }

    #[test]
    fn parse_ignores_blank_lines() {
        let text = r#"

0223_DD34 6008_0180

0223_DD38 309C_1C28

"#;

        let cheats = cheat_map_from_str(text).unwrap();

        // A blank line between the two lines splits them into two codes.
        assert_eq!(cheats.len(), 2);
        assert_eq!(cheats[0].code, vec![0x0223_DD34, 0x6008_0180]);
        assert_eq!(cheats[1].code, vec![0x0223_DD38, 0x309C_1C28]);
    }

    #[test]
    fn to_string_roundtrip() {
        let cheats: CheatMap = vec![
            ArCode { code: vec![0x0223_DD34, 0x6008_0180], enabled: true },
            ArCode { code: vec![0x0223_DD38, 0x309C_1C28], enabled: false },
        ];

        let text = cheat_map_to_string(&cheats);

        assert_eq!(
            text,
            "\
0223_DD34 6008_0180

// disabled
0223_DD38 309C_1C28
"
        );

        let parsed = cheat_map_from_str(&text).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].code, cheats[0].code);
        assert_eq!(parsed[0].enabled, cheats[0].enabled);
        assert_eq!(parsed[1].code, cheats[1].code);
        assert_eq!(parsed[1].enabled, cheats[1].enabled);
    }

    #[test]
    fn parse_invalid_format() {
        let text = "0223_DD34";

        assert!(matches!(cheat_map_from_str(text), Err(CheatMapParseError::InvalidFormat(1))));
    }

    #[test]
    fn parse_invalid_hex() {
        let text = "0223_DD34 ZZZZ_0180";

        assert!(matches!(cheat_map_from_str(text), Err(CheatMapParseError::InvalidHex(1, _))));
    }

    #[test]
    fn empty_string() {
        let cheats = cheat_map_from_str("").unwrap();
        assert!(cheats.is_empty());
        assert_eq!(cheat_map_to_string(&cheats), "");
    }

    #[test]
    fn parse_ignores_inline_comments() {
        let text = r#"
// first
0223_DD34 6008_0180 // patch
0223_DD38 309C_1C28 // another patch
"#;

        let cheats = cheat_map_from_str(text).unwrap();

        assert_eq!(cheats.len(), 1);
        assert_eq!(cheats[0].code, vec![0x0223_DD34, 0x6008_0180, 0x0223_DD38, 0x309C_1C28]);
    }
}
