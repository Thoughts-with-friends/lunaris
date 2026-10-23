//! System > Setup cheat codes, laid out like `docs/design/cheat_gui.png`:
//! the game's info on the left, its cheats (enabled, name, type) in the
//! middle and the selected cheat's name / memo / code on the right.
//!
//! ```text
//!  open    -> cheats/<rom>.json (working copy in this component)
//!  row     -> its name / memo / code go to the editor (an edit in progress
//!             is kept if its code is valid)
//!  Add / Save / Delete / checkboxes -> the working copy
//!  OK      -> write the file, Frontend::load_cheats (runs the enabled codes)
//!  Cancel  -> close, nothing written
//! ```
use bytes::Bytes;
use ds_common::{
    cheats::{self, Cheat},
    frontend::{file_name, lock},
    rom_info::{ICON_SIZE, RomInfo},
};
use freya::{elements::image::ImageHandle, engine::prelude::AlphaType, prelude::*};

use crate::{
    panels::rom_icon,
    state::Ctx,
    theme::{self, ACCENT, DANGER, TEXT, TEXT_DIM, TEXT_FAINT},
};

const LIST_HEIGHT: f32 = 430.0;

/// Why `code` cannot run, if it cannot.
fn code_error(code: &str) -> Option<String> {
    ds_emu::parse_ar_code(code).err()
}

fn human_size(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    if mib >= 1024.0 { format!("{:.1} GiB", mib / 1024.0) } else { format!("{mib:.1} MiB") }
}

#[derive(PartialEq)]
pub struct CheatsView;

impl Component for CheatsView {
    fn render(&self) -> impl IntoElement {
        let ctx = use_consume::<Ctx>();
        let rom = lock(&ctx.front.rom).clone();
        let Some(rom) = rom else {
            return rect().child("No game is running.").into_element();
        };
        let path = cheats::cheats_path(&ctx.front.instance, &rom);

        let load_path = path.clone();
        let mut list = use_state(move || cheats::load(&load_path));
        let mut selected = use_state(|| None::<usize>);
        let mut name = use_state(String::new);
        let mut memo = use_state(String::new);
        let mut code = use_state(String::new);
        let mut error = use_state(|| None::<String>);

        // Select the first cheat once.
        use_hook(move || {
            if let Some(first) = list.peek().first().cloned() {
                selected.set(Some(0));
                name.set(first.name);
                memo.set(first.memo);
                code.set(first.code);
            }
        });

        let info_rom = rom.clone();
        let info = use_hook(move || RomInfo::read(&info_rom).ok());

        let current = selected().and_then(|i| list.read().get(i).cloned());
        let draft_error = current.as_ref().and_then(|_| code_error(&code.read()));
        let edited = current.as_ref().is_some_and(|c| {
            c.name != *name.read() || c.memo != *memo.read() || c.code != *code.read()
        });

        // The working list with a valid edit of the selected cheat written in.
        let with_draft = move || -> Vec<Cheat> {
            let mut all = list.peek().clone();
            if let Some(i) = *selected.peek()
                && let Some(c) = all.get_mut(i)
                && code_error(&code.peek()).is_none()
            {
                c.name.clone_from(&name.peek());
                c.memo.clone_from(&memo.peek());
                c.code.clone_from(&code.peek());
            }
            all
        };
        let mut select = move |next: Vec<Cheat>, index: Option<usize>| {
            let cheat = index.and_then(|i| next.get(i).cloned()).unwrap_or_default();
            list.set(next);
            selected.set(index);
            name.set(cheat.name);
            memo.set(cheat.memo);
            code.set(cheat.code);
        };

        let on_add = move |_| {
            let mut next = with_draft();
            next.push(Cheat { name: "New cheat".to_owned(), ..Cheat::default() });
            let last = next.len() - 1;
            select(next, Some(last));
        };
        let on_save = move |_| {
            let next = with_draft();
            let index = *selected.peek();
            select(next, index);
        };
        let on_delete = move |_| {
            let Some(i) = *selected.peek() else { return };
            let mut next = list.peek().clone();
            next.remove(i);
            let index = if next.is_empty() { None } else { Some(i.min(next.len() - 1)) };
            select(next, index);
        };
        let ok_ctx = ctx.clone();
        let on_ok = move |_| {
            let all = with_draft();
            if let Some(bad) = all.iter().find(|c| c.enabled && c.words().is_err()) {
                error.set(Some(format!("\"{}\": its code is not valid", bad.name)));
                return;
            }
            match cheats::save(&path, &all) {
                Ok(()) => {
                    ok_ctx.front.load_cheats();
                    let mut dialog = ok_ctx.dialog;
                    dialog.set(None);
                }
                Err(e) => error.set(Some(format!("Cannot save the cheats: {e}"))),
            }
        };
        let mut dialog = ctx.dialog;

        let rows: Vec<Cheat> = list.read().clone();
        let can_save = edited && draft_error.is_none();

        rect()
            .width(Size::fill())
            .spacing(14.)
            .child(
                rect().horizontal().spacing(16.).child(info_panel(info, &rom)).child(
                    rect()
                        .width(Size::flex(1.))
                        .spacing(10.)
                        .child(
                            rect()
                                .horizontal()
                                .content(Content::Flex)
                                .cross_align(Alignment::Center)
                                .width(Size::fill())
                                .spacing(8.)
                                .child(
                                    rect()
                                        .width(Size::flex(1.))
                                        .color(theme::rgb(TEXT_DIM))
                                        .font_size(13.)
                                        .child("Available cheats:"),
                                )
                                .child(
                                    Button::new()
                                        .compact()
                                        .enabled(can_save)
                                        .on_press(on_save)
                                        .child("Save"),
                                )
                                .child(
                                    Button::new()
                                        .compact()
                                        .enabled(current.is_some())
                                        .on_press(on_delete)
                                        .child("Delete"),
                                )
                                .child(
                                    Button::new()
                                        .compact()
                                        .filled()
                                        .on_press(on_add)
                                        .child("Add cheat"),
                                ),
                        )
                        .child(
                            rect()
                                .horizontal()
                                .spacing(14.)
                                .child(cheat_list(rows, selected(), with_draft, select, list))
                                .child(editor(current.is_some(), name, memo, code, draft_error)),
                        ),
                ),
            )
            .maybe_child(
                error
                    .read()
                    .clone()
                    .map(|e| rect().color(theme::rgb(DANGER)).font_size(12.5).child(e)),
            )
            .child(
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .main_align(Alignment::End)
                    .spacing(8.)
                    .child(Button::new().on_press(move |_| dialog.set(None)).child("Cancel"))
                    .child(Button::new().filled().on_press(on_ok).child("OK")),
            )
            .into_element()
    }
}

/// Left: the game.
fn info_panel(info: Option<RomInfo>, rom: &std::path::Path) -> Element {
    let icon = info.as_ref().filter(|i| !i.icon_rgba.is_empty()).and_then(|i| {
        ImageHandle::from_rgba(
            ICON_SIZE as u32,
            ICON_SIZE as u32,
            Bytes::from(i.icon_rgba.clone()),
            AlphaType::Unpremul,
        )
    });
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(i) = &info {
        let title =
            i.title_ja.clone().or_else(|| i.title_en.clone()).unwrap_or_else(|| i.title.clone());
        rows.push(("Name", title.lines().next().unwrap_or_default().to_owned()));
        rows.push(("Game code", i.game_code.clone()));
        rows.push(("Format", "NDS".to_owned()));
        rows.push(("Size", human_size(i.file_size)));
    }
    rows.push(("Filepath", file_name(rom)));

    rect()
        .width(Size::px(230.))
        .padding(14.)
        .spacing(10.)
        .corner_radius(12.)
        .background(theme::panel())
        .border(Border::new().width(1.).fill(theme::hairline()))
        .cross_align(Alignment::Center)
        .child(rect().font_size(12.).color(theme::rgb(TEXT_FAINT)).child("Info"))
        .child(rom_icon(icon, 96.))
        .children(rows.into_iter().enumerate().map(|(i, (k, v))| {
            rect()
                .key(i)
                .width(Size::fill())
                .spacing(3.)
                .child(rect().font_size(11.).color(theme::rgb(TEXT_FAINT)).child(k))
                .child(
                    rect()
                        .width(Size::fill())
                        .padding((6., 9.))
                        .corner_radius(7.)
                        .background(theme::surface())
                        .font_size(12.5)
                        .child(label().text(v).max_lines(1).text_overflow(TextOverflow::Ellipsis)),
                )
                .into_element()
        }))
        .into_element()
}

/// Middle: the table.
fn cheat_list(
    rows: Vec<Cheat>,
    selected: Option<usize>,
    with_draft: impl Fn() -> Vec<Cheat> + Clone + 'static,
    select: impl FnMut(Vec<Cheat>, Option<usize>) + Clone + 'static,
    mut list: State<Vec<Cheat>>,
) -> Element {
    let header = rect()
        .horizontal()
        .content(Content::Flex)
        .width(Size::fill())
        .padding((8., 10.))
        .background(theme::rgba(ACCENT, 34))
        .font_size(12.)
        .font_weight(FontWeight::BOLD)
        .color(theme::rgb(TEXT))
        .child(rect().width(Size::px(30.)))
        .child(rect().width(Size::flex(1.)).child("Name"))
        .child(rect().width(Size::px(100.)).child("Type"));

    let empty = rows.is_empty();
    rect()
        .width(Size::px(400.))
        .height(Size::px(LIST_HEIGHT))
        .corner_radius(12.)
        .overflow(Overflow::Clip)
        .background(theme::panel())
        .border(Border::new().width(1.).fill(theme::hairline()))
        .child(header)
        .child(ScrollView::new().height(Size::fill()).children(rows.into_iter().enumerate().map(
            |(i, cheat)| {
                let with_draft = with_draft.clone();
                let mut select = select.clone();
                CheatRow {
                    index: i,
                    cheat,
                    selected: selected == Some(i),
                    on_toggle: EventHandler::new(move |()| {
                        if let Some(c) = list.write().get_mut(i) {
                            c.enabled ^= true;
                        }
                    }),
                    on_select: EventHandler::new(move |()| {
                        if selected != Some(i) {
                            select(with_draft(), Some(i));
                        }
                    }),
                }
                .into_element()
            },
        )))
        .maybe_child(empty.then(|| {
            rect()
                .width(Size::fill())
                .padding(20.)
                .center()
                .font_size(12.5)
                .color(theme::rgb(TEXT_FAINT))
                .child("No cheats yet. Add one with \"Add cheat\".")
        }))
        .into_element()
}

#[derive(PartialEq)]
struct CheatRow {
    index: usize,
    cheat: Cheat,
    selected: bool,
    on_toggle: EventHandler<()>,
    on_select: EventHandler<()>,
}

impl Component for CheatRow {
    fn render(&self) -> impl IntoElement {
        let mut hover = use_state(|| false);
        let on_toggle = self.on_toggle.clone();
        let on_select = self.on_select.clone();
        let background = if self.selected {
            theme::rgba(ACCENT, 110)
        } else if hover() {
            Color::from_argb(22, 255, 255, 255)
        } else if self.index % 2 == 1 {
            Color::from_argb(8, 255, 255, 255)
        } else {
            Color::TRANSPARENT
        };
        rect()
            .key(self.index)
            .horizontal()
            .content(Content::Flex)
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .padding((6., 10.))
            .background(background)
            .font_size(12.5)
            .color(theme::rgb(if self.selected { TEXT } else { TEXT_DIM }))
            .on_pointer_enter(move |_| hover.set(true))
            .on_pointer_leave(move |_| hover.set(false))
            .on_press(move |_| on_select.call(()))
            .child(
                rect()
                    .width(Size::px(30.))
                    .on_press(move |e: Event<PressEventData>| {
                        e.stop_propagation();
                        on_toggle.call(());
                    })
                    .child(Checkbox::new().selected(self.cheat.enabled).size(16.)),
            )
            .child(
                rect().width(Size::flex(1.)).child(
                    label()
                        .text(self.cheat.name.clone())
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis),
                ),
            )
            .child(rect().width(Size::px(100.)).child(self.cheat.kind.clone()))
    }
}

/// Right: name, memo and code of the selected cheat.
fn editor(
    enabled: bool,
    name: State<String>,
    memo: State<String>,
    code: State<String>,
    code_error: Option<String>,
) -> Element {
    let caption = |t: &str| rect().font_size(12.).color(theme::rgb(TEXT_DIM)).child(t.to_owned());
    rect()
        .width(Size::flex(1.))
        .spacing(6.)
        .child(caption("Name:"))
        .child(Input::new(name).enabled(enabled).width(Size::fill()))
        .child(caption("Memo:"))
        .child(
            Input::new(memo)
                .enabled(enabled)
                .multiline(true)
                .width(Size::fill())
                .height(Size::px(170.)),
        )
        .child(caption("Code:"))
        .child(
            rect().font_family("Consolas").child(
                Input::new(code)
                    .enabled(enabled)
                    .multiline(true)
                    .placeholder("02000000 00000063")
                    .width(Size::fill())
                    .height(Size::px(130.)),
            ),
        )
        .child(
            rect()
                .height(Size::px(16.))
                .font_size(11.5)
                .color(theme::rgb(if code_error.is_some() { DANGER } else { TEXT_FAINT }))
                .child(
                    code_error
                        .unwrap_or_else(|| "Action Replay: XXXXXXXX YYYYYYYY per line".to_owned()),
                ),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_checked_like_the_emulator() {
        assert!(code_error("02000000 00000001").is_none());
        assert!(code_error("0200 1").is_some());
        assert_eq!(human_size(2_362_232_012), "2.2 GiB");
    }
}
