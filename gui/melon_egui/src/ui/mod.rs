//! Everything the user sees, and nothing that happens after they click.
//!
//! A module here draws, reads input, and at most *reports* what was asked for:
//! [`menu::bar`] returns an [`Action`](menu::Action), and
//! [`crate::app::MelonEgui::apply`] carries it out.
//!
//! | module      | what it draws                                              |
//! |-------------|------------------------------------------------------------|
//! | `frame`     | one repaint, in order — **start reading here**             |
//! | `menu`      | the menu bar                                               |
//! | `screen`    | the first console's picture (software and OpenGL routes)   |
//! | `layout`    | framebuffer → texture → window, and click → touchscreen    |
//! | `view`      | where the two screens go (View menu options, pure maths)   |
//! | `window`    | the second console's window, second view, `--shot`         |
//! | `panes`     | the settings / tool windows                                |
//! | `osd`       | messages and FPS over the picture                          |
//! | `notice`    | a message's severity → colour and log level                |

pub mod frame;
pub mod layout;
pub mod menu;
pub mod notice;
pub mod osd;
pub mod panes;
pub mod screen;
pub mod view;
pub mod window;
