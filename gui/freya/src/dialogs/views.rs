//! The simpler dialogs: ROM info, Emu / Input / Audio / Wifi / Path
//! settings, About and Multiplayer (same content as the egui GUI).
use std::path::PathBuf;

use bytes::Bytes;
use ds_common::{
    Settings, format_mac,
    frontend::lock,
    parse_mac,
    rom_info::{ICON_SIZE, RomInfo},
};
use freya::{elements::image::ImageHandle, engine::prelude::AlphaType, prelude::*};
use rfd::AsyncFileDialog;

use super::{field, note, section};
use crate::{
    panels::rom_icon,
    state::{Ctx, NO_MULTIPLAYER},
    theme::{self, ACCENT, DANGER, TEXT, TEXT_DIM, TEXT_FAINT},
};

/// Monospace text that can be selected and copied.
fn mono(text: String) -> Element {
    rect().font_family("Consolas").font_size(12.5).child(label().text(text)).into_element()
}

#[derive(PartialEq)]
pub struct RomInfoView;

impl Component for RomInfoView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let rom = ctx.status.read().rom_path.clone();
        let info =
            use_hook(move || rom.as_deref().map(|r| RomInfo::read(r).map_err(|e| e.to_string())));
        let Some(info) = info else {
            return rect().child("No game is running.").into_element();
        };
        let info = match info {
            Ok(info) => info,
            Err(e) => return rect().color(theme::rgb(DANGER)).child(e).into_element(),
        };
        let icon = (!info.icon_rgba.is_empty())
            .then(|| {
                ImageHandle::from_rgba(
                    ICON_SIZE as u32,
                    ICON_SIZE as u32,
                    Bytes::from(info.icon_rgba.clone()),
                    AlphaType::Unpremul,
                )
            })
            .flatten();
        let titles = [info.title_ja.clone(), info.title_en.clone()].into_iter().flatten();
        let binary = |b: ds_common::rom_info::Binary| {
            format!(
                "ROM {:08X}  entry {:08X}  RAM {:08X}  size {:08X}",
                b.rom_offset, b.entry, b.ram_address, b.size
            )
        };

        rect()
            .width(Size::fill())
            .child(
                rect()
                    .horizontal()
                    .spacing(16.)
                    .cross_align(Alignment::Center)
                    .margin((0., 0., 8., 0.))
                    .child(rom_icon(icon, 64.))
                    .child(rect().spacing(2.).children(titles.enumerate().map(|(i, t)| {
                        rect()
                            .key(i)
                            .font_size(if i == 0 { 14. } else { 12.5 })
                            .color(theme::rgb(if i == 0 { TEXT } else { TEXT_DIM }))
                            .child(t)
                            .into_element()
                    }))),
            )
            .child(section("Header"))
            .child(field("Title", mono(info.title.clone())))
            .child(field("Game code", mono(info.game_code.clone())))
            .child(field("Maker code", mono(info.maker_code.clone())))
            .child(field("Version", mono(info.version.to_string())))
            .child(field("File size", mono(format!("{} bytes", info.file_size))))
            .child(field("Capacity", mono(format!("{} MiB", info.capacity / (1024 * 1024)))))
            .child(section("Binaries"))
            .child(field("ARM9", mono(binary(info.arm9))))
            .child(field("ARM7", mono(binary(info.arm7))))
            .child(section("File"))
            .child(field("Path", label().text(info.path).font_size(12.)))
            .into_element()
    }
}

/// Saves `settings.json` after `change`.
fn update_settings(ctx: &Ctx, change: impl FnOnce(&mut Settings)) {
    let mut settings = lock(&ctx.front.settings);
    change(&mut settings);
    settings.save(&ctx.front.instance.settings_path());
}

/// A file or folder with Browse / Clear.
#[derive(PartialEq)]
struct PathField {
    label: &'static str,
    value: Option<PathBuf>,
    folder: bool,
    placeholder: &'static str,
    on_change: EventHandler<Option<PathBuf>>,
}

impl Component for PathField {
    fn render(&self) -> impl IntoElement {
        let on_browse = {
            let on_change = self.on_change.clone();
            let folder = self.folder;
            move |_| {
                let on_change = on_change.clone();
                spawn(async move {
                    let dialog = AsyncFileDialog::new();
                    let picked =
                        if folder { dialog.pick_folder().await } else { dialog.pick_file().await };
                    if let Some(p) = picked {
                        on_change.call(Some(p.path().to_owned()));
                    }
                });
            }
        };
        let on_clear = {
            let on_change = self.on_change.clone();
            move |_| on_change.call(None)
        };
        let text = self
            .value
            .as_ref()
            .map_or_else(|| self.placeholder.to_owned(), |p| p.display().to_string());
        field(
            self.label,
            rect()
                .horizontal()
                .content(Content::Flex)
                .cross_align(Alignment::Center)
                .width(Size::fill())
                .spacing(6.)
                .child(
                    rect()
                        .width(Size::flex(1.))
                        .padding((7., 10.))
                        .corner_radius(8.)
                        .background(theme::surface())
                        .color(theme::rgb(if self.value.is_some() { TEXT } else { TEXT_FAINT }))
                        .child(
                            label().text(text).max_lines(1).text_overflow(TextOverflow::Ellipsis),
                        ),
                )
                .child(Button::new().compact().on_press(on_browse).child("Browse…"))
                .maybe_child(
                    self.value
                        .is_some()
                        .then(|| Button::new().compact().flat().on_press(on_clear).child("Clear")),
                ),
        )
    }
}

/// Small selectable pills (a compact radio group).
fn pills<T: Clone + PartialEq + 'static>(
    options: Vec<(T, String)>,
    selected: &T,
    on_pick: impl Fn(T) + Clone + 'static,
) -> Element {
    rect()
        .horizontal()
        .spacing(6.)
        .children(options.into_iter().enumerate().map(|(i, (value, text))| {
            let on = &value == selected;
            let on_pick = on_pick.clone();
            rect()
                .key(i)
                .padding((5., 12.))
                .corner_radius(14.)
                .background(if on { theme::rgba(ACCENT, 90) } else { theme::surface() })
                .border(Border::new().width(1.).fill(if on {
                    theme::rgba(ACCENT, 200)
                } else {
                    theme::hairline()
                }))
                .color(theme::rgb(if on { TEXT } else { TEXT_DIM }))
                .font_size(12.)
                .on_press(move |_| on_pick(value.clone()))
                .child(text)
                .into_element()
        }))
        .into_element()
}

#[derive(PartialEq)]
pub struct EmuSettingsView;

impl Component for EmuSettingsView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        // Re-read after every change.
        let version = use_state(|| 0_u32);
        let _ = *version.read();
        let s = lock(&ctx.front.settings).clone();

        let set = {
            let ctx = ctx;
            move |change: Box<dyn FnOnce(&mut Settings)>| {
                update_settings(&ctx, change);
                let mut version = version;
                *version.write() += 1;
            }
        };
        let path = |label: &'static str, value: Option<PathBuf>, which: u8| {
            let set = set.clone();
            PathField {
                label,
                value,
                folder: false,
                placeholder: "(built-in)",
                on_change: EventHandler::new(move |p: Option<PathBuf>| {
                    set(Box::new(move |s: &mut Settings| match which {
                        0 => s.arm9_bios_path = p,
                        1 => s.arm7_bios_path = p,
                        _ => s.firmware_path = p,
                    }));
                }),
            }
        };
        let hle = {
            let set = set.clone();
            move |_| set(Box::new(|s: &mut Settings| s.hle_bios ^= true))
        };
        let frameskip = s.frameskip;

        rect()
            .width(Size::fill())
            .child(section("BIOS and firmware"))
            .child(field(
                "High-level BIOS",
                rect()
                    .horizontal()
                    .spacing(10.)
                    .cross_align(Alignment::Center)
                    .child(Switch::new().toggled(s.hle_bios).on_toggle(hle))
                    .child(rect().color(theme::rgb(TEXT_DIM)).font_size(12.).child("No BIOS dump needed")),
            ))
            .child(path("ARM9 BIOS", s.arm9_bios_path.clone(), 0))
            .child(path("ARM7 BIOS", s.arm7_bios_path.clone(), 1))
            .child(path("Firmware", s.firmware_path.clone(), 2))
            .child(section("Emulation"))
            .child(field(
                "Frame skip",
                pills(
                    (0..=9).map(|n| (n, n.to_string())).collect(),
                    &frameskip,
                    {
                        let set = set.clone();
                        move |n| set(Box::new(move |s: &mut Settings| s.frameskip = n))
                    },
                ),
            ))
            .child(field(
                "Log level",
                pills(
                    ["error", "warn", "info", "debug", "trace"]
                        .into_iter()
                        .map(|l| (l.to_owned(), l.to_owned()))
                        .collect(),
                    &s.log_level,
                    move |l| set(Box::new(move |s: &mut Settings| s.log_level = l)),
                ),
            ))
            .child(note(
                "Saved to settings.json; applies when a game is next started or reset (log level: on restart).",
            ))
    }
}

#[derive(PartialEq)]
pub struct InputView;

impl Component for InputView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let gamepads = ctx.gamepads.read().clone();
        let rows = [
            ("A", "X", "Right face button"),
            ("B", "Z", "Bottom face button"),
            ("X", "S", "Top face button"),
            ("Y", "A", "Left face button"),
            ("L / R", "Q / W", "Shoulders / triggers"),
            ("START / SELECT", "Enter / Backspace", "Start / Back"),
            ("D-pad", "Arrow keys", "D-pad / left stick"),
            ("Touch screen", "Mouse (left button)", "-"),
        ];
        let hotkeys = [
            ("Ctrl+O", "Open ROM"),
            ("Ctrl+P", "Pause / resume"),
            ("Ctrl+R", "Reset"),
            ("Ctrl+N", "Frame step"),
            ("Left stick press", "Next emulation speed (gamepad)"),
            ("Ctrl+Q", "Quit"),
        ];
        let table_row = |i: usize, cells: [&str; 3], head: bool| {
            rect()
                .key(i)
                .horizontal()
                .width(Size::fill())
                .padding((7., 10.))
                .corner_radius(6.)
                .background(if head {
                    theme::rgba(ACCENT, 40)
                } else if i.is_multiple_of(2) {
                    Color::from_argb(14, 255, 255, 255)
                } else {
                    Color::TRANSPARENT
                })
                .font_size(12.5)
                .color(theme::rgb(if head { TEXT } else { TEXT_DIM }))
                .children(cells.into_iter().enumerate().map(|(c, t)| {
                    rect()
                        .key(c)
                        .width(Size::percent([28., 32., 40.][c]))
                        .child(t.to_owned())
                        .into_element()
                }))
                .into_element()
        };
        rect()
            .width(Size::fill())
            .child(section("Buttons"))
            .child(table_row(usize::MAX, ["DS", "Keyboard", "Gamepad"], true))
            .children(rows.iter().enumerate().map(|(i, r)| table_row(i, (*r).into(), false)))
            .child(section("Hotkeys"))
            .children(hotkeys.iter().enumerate().map(|(i, (k, what))| {
                field(k, rect().color(theme::rgb(TEXT)).child(*what)).into_element_with_key(i)
            }))
            .child(section("Gamepads"))
            .child(rect().font_size(12.5).color(theme::rgb(TEXT_DIM)).child(
                if gamepads.is_empty() { "None connected".to_owned() } else { gamepads.join(", ") },
            ))
            .child(note("The mapping is fixed; remapping is not implemented yet."))
    }
}

trait WithKey {
    fn into_element_with_key(self, key: usize) -> Element;
}

impl WithKey for Element {
    fn into_element_with_key(self, key: usize) -> Element {
        rect().key(key).width(Size::fill()).child(self).into_element()
    }
}

#[derive(PartialEq)]
pub struct AudioView;

impl Component for AudioView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let volume = ctx.config.read().volume;
        let set_ctx = ctx;
        rect()
            .width(Size::fill())
            .child(field(
                "Volume",
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .width(Size::fill())
                    .spacing(10.)
                    .child(
                        rect().width(Size::flex(1.)).child(
                            Slider::new(move |v: f64| {
                                set_ctx
                                    .update_config(|c| c.volume = v.round().clamp(0., 100.) as u8);
                            })
                            .value(f64::from(volume))
                            .size(Size::fill()),
                        ),
                    )
                    .child(rect().width(Size::px(40.)).child(format!("{volume}%"))),
            ))
            .child(note("Sound follows the emulation speed (its pitch changes away from 1x)."))
    }
}

#[derive(PartialEq)]
pub struct WifiView;

impl Component for WifiView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let current = lock(&ctx.front.settings).mac_address.clone().unwrap_or_default();
        let mut text = use_state(|| current.clone());
        let valid = parse_mac(&text.read());
        let save_ctx = ctx.clone();
        let random_ctx = ctx.clone();
        rect()
            .width(Size::fill())
            .child(field(
                "Instance",
                rect().child(format!("instance{} (each instance keeps its own MAC)", ctx.front.instance.number())),
            ))
            .child(field(
                "MAC address",
                rect()
                    .horizontal()
                    .spacing(8.)
                    .cross_align(Alignment::Center)
                    .child(Input::new(text).width(Size::px(180.)).placeholder("00:09:BF:12:34:56"))
                    .child(
                        Button::new()
                            .filled()
                            .compact()
                            .enabled(valid.is_some())
                            .on_press(move |_| {
                                if let Some(mac) = valid {
                                    update_settings(&save_ctx, |s| s.mac_address = Some(format_mac(mac)));
                                }
                            })
                            .child("Save"),
                    )
                    .child(
                        Button::new()
                            .compact()
                            .on_press(move |_| {
                                update_settings(&random_ctx, |s| {
                                    s.mac_address = None;
                                    s.ensure_mac();
                                });
                                let mac = lock(&random_ctx.front.settings).mac_address.clone();
                                text.set(mac.unwrap_or_default());
                            })
                            .child("Randomize"),
                    ),
            ))
            .maybe_child(valid.is_none().then(|| {
                rect()
                    .margin((0., 0., 0., 130.))
                    .font_size(12.)
                    .color(theme::rgb(DANGER))
                    .child("Format: 00:09:BF:12:34:56")
            }))
            .child(note(
                "Consoles playing together need different MACs. Takes effect on the next start/reset.",
            ))
    }
}

#[derive(PartialEq)]
pub struct PathsView;

impl Component for PathsView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let dir = ctx.config.read().rom_dir.clone();
        let set_ctx = ctx.clone();
        let root = std::path::absolute(ctx.front.instance.root())
            .unwrap_or_else(|_| ctx.front.instance.root().to_owned());
        let open_root = root.clone();
        let error_ctx = ctx;
        rect()
            .width(Size::fill())
            .child(PathField {
                label: "ROM folder",
                value: (!dir.as_os_str().is_empty()).then_some(dir),
                folder: true,
                placeholder: "(not set)",
                on_change: EventHandler::new(move |p: Option<PathBuf>| {
                    set_ctx.update_config(|c| c.rom_dir = p.unwrap_or_default());
                }),
            })
            .child(section("This instance"))
            .child(field(
                "Data folder",
                rect()
                    .horizontal()
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .width(Size::fill())
                    .spacing(8.)
                    .child(rect().width(Size::flex(1.)).child(mono(root.display().to_string())))
                    .child(
                        Button::new()
                            .compact()
                            .on_press(move |_| {
                                if let Err(e) = ds_common::reveal(&open_root) {
                                    error_ctx.show_error(format!(
                                        "Cannot open {}: {e}",
                                        open_root.display()
                                    ));
                                }
                            })
                            .child("Open"),
                    ),
            ))
            .child(note(
                "Saves, settings, cheats and logs of this instance live in the data folder.",
            ))
    }
}

#[derive(PartialEq)]
pub struct AboutView;

impl Component for AboutView {
    fn render(&self) -> impl IntoElement {
        rect()
            .width(Size::fill())
            .cross_align(Alignment::Center)
            .spacing(8.)
            .child(crate::titlebar::logo(56.))
            .child(rect().font_size(20.).font_weight(FontWeight::BOLD).child("Lunaris DS"))
            .child(
                rect()
                    .color(theme::rgb(TEXT_DIM))
                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
            )
            .child(rect().child("A Nintendo DS emulator written in Rust."))
            .child(
                rect()
                    .font_size(12.)
                    .color(theme::rgb(TEXT_FAINT))
                    .cross_align(Alignment::Center)
                    .child("Core based on CorgiDS; Wi-Fi and multiplayer after melonDS.")
                    .child("Menus laid out after melonDS. GUI built with freya. License: GPL-3.0"),
            )
            .child(
                rect()
                    .color(theme::rgb(ACCENT))
                    .font_size(12.5)
                    .child("github.com/Thoughts-with-friends/cogids-rs"),
            )
    }
}

#[derive(PartialEq)]
pub struct MultiplayerView;

impl Component for MultiplayerView {
    fn render(&self) -> impl IntoElement {
        rect().width(Size::fill()).spacing(8.).child(NO_MULTIPLAYER).child(note(
            "System > Multiplayer > Launch new instance starts another emulator window.",
        ))
    }
}
