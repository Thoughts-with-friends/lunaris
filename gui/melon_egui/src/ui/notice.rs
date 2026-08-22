//! How a message to the user is coloured, and how it reaches the log.
//!
//! One vocabulary for both destinations. Every outcome a command reports —
//! on the OSD, in a pane, or in `logs/melon_egui.log` — picks a [`Severity`],
//! and the colour and the log level both follow from it. That is what keeps a
//! failure red on screen *and* an `error!` in the log without either call site
//! having to remember both.

use egui::Color32;

/// What a message is telling the user.
///
/// Deliberately not `log::Level`: the UI needs a *success* that the `log`
/// crate has no name for, and does not need `Debug` or `Trace`, which never
/// reach the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Severity {
    /// Something the user asked for did not happen. Red.
    Error,
    /// It happened, but not as asked, or something is about to go wrong.
    /// Yellow.
    Warn,
    /// It happened. Green.
    Success,
    /// Neither good nor bad — a state change worth mentioning. Default colour.
    #[default]
    Info,
}

impl Severity {
    /// The text colour, against `dark` or light chrome.
    ///
    /// `Info` follows the theme rather than picking a colour, so that ordinary
    /// text stays ordinary and the three that matter stand out.
    #[must_use]
    pub const fn color(self, dark: bool) -> Color32 {
        match self {
            Self::Error => Color32::from_rgb(0xE0, 0x4C, 0x4C),
            Self::Warn => Color32::from_rgb(0xD8, 0xA4, 0x1E),
            Self::Success => Color32::from_rgb(0x3F, 0xB9, 0x50),
            Self::Info if dark => Color32::WHITE,
            Self::Info => Color32::BLACK,
        }
    }

    /// The log level a message of this severity is recorded at.
    #[must_use]
    pub const fn level(self) -> log::Level {
        match self {
            Self::Error => log::Level::Error,
            Self::Warn => log::Level::Warn,
            Self::Success | Self::Info => log::Level::Info,
        }
    }
}

/// A message and how to show it.
#[derive(Clone, Debug)]
pub struct Notice {
    pub severity: Severity,
    pub text: String,
}

impl Notice {
    /// Build one and record it, so no site has to remember to do both.
    pub fn new(severity: Severity, text: impl Into<String>) -> Self {
        let text = text.into();
        log::log!(severity.level(), "{text}");
        Self { severity, text }
    }

    /// Build one without logging it.
    ///
    /// For a status line derived afresh each repaint: it says the same thing
    /// every frame, so logging it would bury everything else.
    pub fn quiet(severity: Severity, text: impl Into<String>) -> Self {
        Self { severity, text: text.into() }
    }

    /// Draw it as one line of coloured text.
    pub fn show(&self, ui: &mut egui::Ui) -> egui::Response {
        ui.colored_label(self.severity.color(ui.visuals().dark_mode), &self.text)
    }
}

impl Default for Notice {
    fn default() -> Self {
        Self::quiet(Severity::Info, String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::{Notice, Severity};

    #[test]
    fn each_severity_has_its_own_colour_and_info_follows_the_theme() {
        let colours = [Severity::Error, Severity::Warn, Severity::Success]
            .map(|severity| severity.color(true));
        assert_eq!(colours.len(), 3);
        assert!(colours[0] != colours[1] && colours[1] != colours[2]);
        // The one that is not a signal is the one that changes with the theme.
        assert_ne!(Severity::Info.color(true), Severity::Info.color(false));
        for severity in [Severity::Error, Severity::Warn, Severity::Success] {
            assert_eq!(severity.color(true), severity.color(false));
        }
    }

    #[test]
    fn success_is_logged_as_info_because_the_log_crate_has_no_word_for_it() {
        assert_eq!(Severity::Error.level(), log::Level::Error);
        assert_eq!(Severity::Warn.level(), log::Level::Warn);
        assert_eq!(Severity::Success.level(), log::Level::Info);
        assert_eq!(Severity::Info.level(), log::Level::Info);
    }

    #[test]
    fn a_quiet_notice_keeps_its_text_and_severity() {
        let notice = Notice::quiet(Severity::Warn, "no cart loaded");
        assert_eq!(notice.severity, Severity::Warn);
        assert_eq!(notice.text, "no cart loaded");
    }
}
