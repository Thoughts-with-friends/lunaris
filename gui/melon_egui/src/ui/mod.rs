//! Everything the user sees, and nothing that happens after they click.
//!
//! The boundary is the button press. A module in here draws, reads input, and
//! at most *reports* what was asked for — [`menu::bar`] returns an
//! [`Action`](menu::Action) rather than performing one. Carrying it out is
//! [`crate::app::MelonEgui::apply`]'s job, and whatever that touches on disk is
//! [`crate::file`]'s.

pub mod frame;
pub mod layout;
pub mod menu;
pub mod notice;
pub mod osd;
pub mod panes;
pub mod screen;
pub mod view;
pub mod window;
