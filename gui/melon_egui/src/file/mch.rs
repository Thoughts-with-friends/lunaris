//! Action Replay codes: the list, its file, and the text a user pastes in.
//!
//! melonDS runs the enabled codes out of the ARM7's VBlank handler, exactly as
//! the hardware does, so this module never touches the console — it only builds
//! the list `melonds::Nds::set_cheats` takes.
//!
//! # The file
//!
//! Codes live beside the ROM in melonDS's own `.mch` format, so a file written
//! here opens in melonDS and vice versa. That format is line-based:
//!
//! ```text
//! CAT 0 Category name
//! CODE 1 Code name
//! DESC something about it
//! 020F5CE4 000003E7
//! ```
//!
//! `CAT` opens a category, `CODE <enabled> <name>` a code, `DESC` annotates
//! whichever of the two came last, and every other line is a pair of 32-bit
//! words. `ROOT` returns to the top level. Categories are kept only as a label
//! on each code: this front end shows one flat list, and writes the grouping
//! back out on save.

use std::path::Path;

/// One code, as the dialog and the file both see it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cheat {
    pub name: String,
    pub description: String,
    /// The `CAT` this code was found under, empty for the root.
    pub category: String,
    /// The code itself, as 32-bit words: Action Replay codes are word pairs.
    pub code: Vec<u32>,
    pub enabled: bool,
}

impl Cheat {
    /// The code as the text a user would paste — the same `%08X %08X` lines the
    /// file holds, which is what every published code list uses.
    pub fn text(&self) -> String {
        self.code
            .chunks(2)
            .map(|pair| match pair {
                [a, b] => format!("{a:08X} {b:08X}"),
                [a] => format!("{a:08X}"),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether the words make a whole number of Action Replay instructions.
    ///
    /// An odd word count is not rejected — melonDS reads what pairs it can —
    /// but it is always a typo, so the dialog says so.
    pub const fn is_well_formed(&self) -> bool {
        !self.code.is_empty() && self.code.len().is_multiple_of(2)
    }

    /// What the core takes.
    pub fn to_core(&self) -> melonds::Cheat {
        melonds::Cheat { name: self.name.clone(), code: self.code.clone(), enabled: self.enabled }
    }
}

/// Parse pasted code text into 32-bit words.
///
/// Takes the forms published code lists actually come in: words separated by
/// spaces, by `:` or `-`, or run together as one 16-hex-digit instruction.
/// `#` and `;` start a comment.
///
/// # Errors
///
/// The offending token, rather than a message, so the caller can say exactly
/// where the paste went wrong.
pub fn parse_code(text: &str) -> Result<Vec<u32>, String> {
    let mut words = Vec::new();
    for line in text.lines() {
        let line = line.split(['#', ';']).next().unwrap_or("");
        // `:` and `-` separate words rather than sit inside one, so they are
        // separators here too.
        for token in line.split([' ', '\t', ':', '-']) {
            if token.is_empty() {
                continue;
            }
            // A whole AR instruction written without a space in it: two words,
            // eight hex digits each.
            let parts: Vec<&str> =
                if token.len() == 16 { vec![&token[..8], &token[8..]] } else { vec![token] };
            for part in parts {
                let word = u32::from_str_radix(part, 16).map_err(|_| token.to_owned())?;
                words.push(word);
            }
        }
    }
    Ok(words)
}

/// Read a melonDS `.mch` file. A missing file is an empty list, not an error:
/// most carts have no codes and the front end looks for one every time.
pub fn load(path: &Path) -> Result<Vec<Cheat>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    Ok(parse_file(&text))
}

/// The `.mch` reader proper, split out so it can be tested without a file.
pub fn parse_file(text: &str) -> Vec<Cheat> {
    let mut cheats: Vec<Cheat> = Vec::new();
    let mut category = String::new();
    // Which of the two `DESC` describes, as melonDS tracks it.
    let mut last_was_code = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let upper = line.to_ascii_uppercase();

        if upper == "ROOT" {
            category.clear();
            last_was_code = false;
        } else if let Some(rest) = strip_keyword(line, "CAT") {
            // `CAT <onlyone> <name>`, or just `CAT <name>` in older files.
            category = rest
                .split_once(' ')
                .filter(|(flag, _)| *flag == "0" || *flag == "1")
                .map_or(rest, |(_, name)| name)
                .trim()
                .to_owned();
            last_was_code = false;
        } else if let Some(rest) = strip_keyword(line, "CODE") {
            let (enabled, name) = rest.split_once(' ').unwrap_or((rest, ""));
            cheats.push(Cheat {
                name: name.trim().to_owned(),
                description: String::new(),
                category: category.clone(),
                code: Vec::new(),
                enabled: enabled == "1",
            });
            last_was_code = true;
        } else if let Some(rest) = strip_keyword(line, "DESC") {
            if last_was_code && let Some(cheat) = cheats.last_mut() {
                cheat.description = rest.trim().to_owned();
            }
        } else if let Some(cheat) = cheats.last_mut() {
            // Anything else is data for the code that was opened last. A line
            // that will not parse is dropped rather than failing the file: one
            // bad line should not cost the user every other code in it.
            if let Ok(words) = parse_code(line) {
                cheat.code.extend(words);
            }
        }
    }
    cheats
}

/// `line` without a leading `keyword` (case-insensitively), or `None`.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let (head, rest) = line.split_at_checked(keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = rest.strip_prefix(' ')?;
    Some(rest)
}

/// Write the list back out in `.mch` form, in the order it is held.
///
/// The order is the file's, not a regrouping of it: the dialog lets codes be
/// dragged into whatever order the user wants, and a writer that sorted them
/// back under their categories would undo that the moment it ran. A `CAT` (or
/// `ROOT`) line is emitted whenever the category changes from one code to the
/// next, which is exactly how the format expresses a run of codes under a
/// heading -- so a list that alternates between two categories writes that
/// heading twice, and reads back as the same order it was written in.
///
/// # Errors
///
/// The file could not be written.
pub fn save(path: &Path, cheats: &[Cheat]) -> Result<(), String> {
    let mut text = String::new();
    // `None` until the first code, so the opening heading is always written
    // even when that code is uncategorised.
    let mut current: Option<&str> = None;

    for cheat in cheats {
        if current != Some(cheat.category.as_str()) {
            if cheat.category.is_empty() {
                text.push_str("ROOT\n\n");
            } else {
                text.push_str(&format!("CAT 0 {}\n\n", cheat.category));
            }
            current = Some(&cheat.category);
        }
        write_cheat(&mut text, cheat);
    }

    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn write_cheat(text: &mut String, cheat: &Cheat) {
    text.push_str(&format!("CODE {} {}\n", u8::from(cheat.enabled), cheat.name));
    if !cheat.description.is_empty() {
        text.push_str(&format!("DESC {}\n", cheat.description));
    }
    text.push_str(&cheat.text());
    text.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::{Cheat, parse_code, parse_file, save};

    #[test]
    fn a_pasted_code_takes_the_forms_code_lists_are_published_in() {
        assert_eq!(parse_code("020F5CE4 000003E7").unwrap(), vec![0x020F_5CE4, 0x0000_03E7]);
        assert_eq!(parse_code("020F5CE4:000003E7").unwrap(), vec![0x020F_5CE4, 0x0000_03E7]);
        assert_eq!(parse_code("020F5CE4000003E7").unwrap(), vec![0x020F_5CE4, 0x0000_03E7]);
        assert_eq!(
            parse_code("020F5CE4 000003E7 # max money\n").unwrap(),
            vec![0x020F_5CE4, 0x0000_03E7]
        );
        assert_eq!(parse_code("").unwrap(), Vec::<u32>::new());
    }

    #[test]
    fn a_word_that_is_not_hex_names_itself() {
        assert_eq!(parse_code("020F5CE4 nonsense").unwrap_err(), "nonsense");
    }

    #[test]
    fn a_melonds_file_round_trips() {
        let text = "CAT 0 Money\n\nCODE 1 Max money\nDESC as much as it holds\n\
                    020F5CE4 000003E7\n\nCODE 0 Off by default\n020F5CE8 00000001\n";
        let cheats = parse_file(text);
        assert_eq!(cheats.len(), 2);
        assert_eq!(cheats[0].name, "Max money");
        assert_eq!(cheats[0].category, "Money");
        assert_eq!(cheats[0].description, "as much as it holds");
        assert!(cheats[0].enabled);
        assert_eq!(cheats[0].code, vec![0x020F_5CE4, 0x0000_03E7]);
        assert!(!cheats[1].enabled, "CODE 0 is a code that is held but not run");

        let dir = std::env::temp_dir().join("melon_egui-cheat-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("codes.mch");
        save(&path, &cheats).unwrap();
        assert_eq!(parse_file(&std::fs::read_to_string(&path).unwrap()), cheats);
    }

    /// Dragging a code up the list has to survive the save, which means the
    /// writer cannot regroup by category -- see [`save`].
    #[test]
    fn the_order_the_list_is_in_is_the_order_the_file_keeps() {
        let cheats = vec![
            Cheat { name: "In a category".into(), category: "Items".into(), ..Cheat::default() },
            Cheat { name: "At the root".into(), ..Cheat::default() },
            Cheat { name: "Back in it".into(), category: "Items".into(), ..Cheat::default() },
        ];
        let dir = std::env::temp_dir().join("melon_egui-cheat-order-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("codes.mch");
        save(&path, &cheats).unwrap();

        let read = parse_file(&std::fs::read_to_string(&path).unwrap());
        let names: Vec<&str> = read.iter().map(|cheat| cheat.name.as_str()).collect();
        assert_eq!(names, ["In a category", "At the root", "Back in it"]);
        assert_eq!(read, cheats, "the categories survive the interleaving too");
    }

    #[test]
    fn an_older_file_without_the_only_one_flag_still_reads() {
        let cheats = parse_file("CAT Money\nCODE 1 Max\n020F5CE4 000003E7\n");
        assert_eq!(cheats[0].category, "Money");
    }

    #[test]
    fn an_odd_number_of_words_is_flagged_rather_than_dropped() {
        let cheat = Cheat { code: vec![0x0200_0000], ..Cheat::default() };
        assert!(!cheat.is_well_formed());
        assert_eq!(cheat.text(), "02000000");
    }
}
