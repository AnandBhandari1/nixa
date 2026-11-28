use super::actions::{self, FileAction};
use super::search::Hit;
use super::tree::{Row, file_name};
use super::{FilesEvent, FilesView, git, tone};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Corner, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Window, div, px, uniform_list,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::menu::{MenuItem, MenuKey, menu, menu_key};
use nixa_ui::modal::{dialog, overlay};
use nixa_ui::popover::{float_at, tooltip};
use nixa_ui::{Icon, Theme, icon, theme};
use std::rc::Rc;

pub const ROW_HEIGHT: f32 = 28.;
pub const INDENT: f32 = 18.;

fn mark_color(t: &Theme, mark: git::Mark) -> gpui::Hsla {
    match mark {
        git::Mark::Added => t.green,
        git::Mark::Deleted | git::Mark::Conflict => t.red,
        git::Mark::Modified => t.yellow,
    }
}

fn row_tip(row: &Row) -> String {
    let what = if row.dir {
        "Click to open folder"
    } else {
        "Click to open in the editor"
    };
    format!("{} · {what} · Right-click for actions", row.name)
}

fn tree_row(v: &FilesView, row: &Row, t: &Theme, cx: &mut Context<FilesView>) -> AnyElement {
    let color = tone::color(t, tone::tone(&row.name, row.dir));
    let secret = !row.dir && tone::sensitive(&row.name);
    let mark = git::mark_for(&v.marks, &row.path);
    let hover = t.hover;
    let path = row.path.clone();
    let dir = row.dir;
    let target = super::target_of(v, &row.path, row.dir);
    let selector = format!("file-{}", row.path);
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .h(px(ROW_HEIGHT))
        .flex_none()
        .pl(px(8. + row.depth as f32 * INDENT))
        .pr(px(8.))
        .gap(px(7.))
        .flex()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(row_tip(row)))
        .on_click(cx.listener(move |v, e: &ClickEvent, _, cx| {
            if !e.standard_click() {
                return;
            }
            if dir {
                super::toggle_folder(v, &path, cx);
            } else {
                super::open_file(v, &path, e.click_count() >= 2, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(target) = target.clone() {
                    super::open_menu(v, target, e.position, window, cx);
                }
            }),
        )
        .child(if row.dir {
            icon(if row.open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            })
            .size(px(12.))
            .flex_none()
            .text_color(t.dim)
            .into_any_element()
        } else {
            div().w(px(12.)).flex_none().into_any_element()
        })
        .child(
            icon(if row.dir { Icon::Folder } else { Icon::Files })
                .size(px(14.))
                .flex_none()
                .text_color(color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(color)
                .when(secret, |el| el.font_weight(FontWeight::MEDIUM))
                .child(SharedString::from(row.name.clone())),
        )
        .when_some(mark, |el, mark| {
            el.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(mark_color(t, mark))
                    .child(mark.letter()),
            )
        })
        .into_any_element()
}

fn hit_row(
    v: &FilesView,
    index: usize,
    hit: &Hit,
    t: &Theme,
    cx: &mut Context<FilesView>,
) -> AnyElement {
    let name = file_name(&hit.path).to_string();
    let color = tone::color(t, tone::tone(&name, false));
    let hover = t.hover;
    let path = hit.path.clone();
    let target = super::target_of(v, &hit.path, false);
    let place = match hit.line {
        Some(line) => format!("{}:{line}", hit.path),
        None => hit.path.clone(),
    };
    let selector = format!("hit-{index}");
    let shown = if hit.line.is_some() {
        let text = hit.text.trim();
        if text.is_empty() {
            " ".to_string()
        } else {
            text.to_string()
        }
    } else {
        name.clone()
    };
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .h(px(44.))
        .flex_none()
        .px(px(10.))
        .py(px(5.))
        .gap(px(8.))
        .flex()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(format!(
            "{place} · Click to open in the editor"
        )))
        .on_click(cx.listener(move |v, e: &ClickEvent, _, cx| {
            if e.standard_click() {
                super::open_file(v, &path, e.click_count() >= 2, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(target) = target.clone() {
                    super::open_menu(v, target, e.position, window, cx);
                }
            }),
        )
        .child(
            icon(Icon::Files)
                .size(px(14.))
                .flex_none()
                .text_color(color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(t.dim)
                        .child(SharedString::from(place)),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .child(SharedString::from(shown)),
                ),
        )
        .into_any_element()
}

fn note(id: &'static str, text: String, color: gpui::Hsla) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex_none()
        .p(px(8.))
        .text_size(px(12.))
        .text_color(color)
        .child(SharedString::from(text))
        .into_any_element()
}

fn header(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> AnyElement {
    let folder = v.root.as_deref().map(file_name).unwrap_or("").to_string();
    div()
        .h(px(42.))
        .flex_none()
        .px(px(14.))
        .pr(px(8.))
        .gap(px(6.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child("Files"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_right()
                .text_size(px(12.))
                .text_color(t.dim)
                .child(SharedString::from(folder)),
        )
        .child(
            icon_button(t, "files-refresh", Icon::Refresh, IconSize::Small, false)
                .debug_selector(|| "files-refresh".into())
                .tooltip(tooltip("Refresh the file list"))
                .on_click(cx.listener(|v, _, _, cx| super::refresh(v, cx))),
        )
        .child(
            icon_button(t, "files-close", Icon::X, IconSize::Small, false)
                .debug_selector(|| "files-close".into())
                .tooltip(tooltip("Close the Files panel"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FilesEvent::Close))),
        )
        .into_any_element()
}

fn search_bar(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> AnyElement {
    let searching = !v.query.trim().is_empty();
    let hover = t.hover;
    div()
        .h(px(44.))
        .flex_none()
        .px(px(10.))
        .gap(px(7.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .id("files-search")
                .debug_selector(|| "files-search".into())
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .tooltip(tooltip("Search file names and contents · Enter to search"))
                .child(v.search.clone()),
        )
        .when(searching, |el| {
            el.child(
                div()
                    .id("files-search-clear")
                    .debug_selector(|| "files-search-clear".into())
                    .w(px(26.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .text_color(t.muted)
                    .hover(move |s| s.bg(hover))
                    .tooltip(tooltip("Clear search"))
                    .on_click(cx.listener(|v, _, _, cx| super::clear_search(v, cx)))
                    .child("×"),
            )
        })
        .into_any_element()
}

fn tree_body(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> AnyElement {
    let count = v.rows.len();
    let list = uniform_list(
        "files-tree",
        count,
        cx.processor(|v: &mut FilesView, range: std::ops::Range<usize>, _, cx| {
            let t = theme(cx).clone();
            range
                .filter_map(|i| v.rows.get(i).cloned())
                .map(|row| tree_row(v, &row, &t, cx))
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(v.scroll.clone())
    .flex_1()
    .min_h_0()
    .p(px(6.));
    let empty = v.tree.entries.get("").is_none_or(Vec::is_empty);
    let text = match (&v.error, &v.root) {
        (Some(e), _) => e.clone(),
        (None, None) => "No folder".into(),
        _ if !v.loaded => "Loading files…".into(),
        _ => "No files".into(),
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .when(!empty, |el| el.child(list))
        .when_some(v.error.clone().filter(|_| !empty), |el, error| {
            el.child(note("files-tree-error", error, t.red))
        })
        .when(empty, |el| {
            let color = if v.error.is_some() { t.red } else { t.dim };
            el.child(note("files-empty", text, color))
        })
        .when(v.tree.limited, |el| {
            el.child(note(
                "files-limited",
                "File list limited to 500 entries per folder and 5,000 total".into(),
                t.dim,
            ))
        })
        .into_any_element()
}

fn search_body(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> AnyElement {
    let names: Vec<Hit> = v.found.names.clone();
    let lines: Vec<Hit> = v.found.lines.clone();
    let none = names.is_empty() && lines.is_empty();
    let mut body = div()
        .id("files-search-results")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p(px(6.))
        .flex()
        .flex_col();
    if !names.is_empty() {
        body = body.child(group(t, "Names"));
        for (i, hit) in names.iter().enumerate() {
            body = body.child(hit_row(v, i, hit, t, cx));
        }
    }
    if !lines.is_empty() {
        body = body.child(group(t, "Contents"));
        for (i, hit) in lines.iter().enumerate() {
            body = body.child(hit_row(v, names.len() + i, hit, t, cx));
        }
    }
    let status = match (&v.search_error, v.searching, none) {
        (Some(e), _, _) => Some((e.clone(), t.red)),
        (None, true, _) => Some(("Searching…".to_string(), t.dim)),
        (None, false, true) => Some(("No matches".to_string(), t.dim)),
        _ => None,
    };
    body.when_some(status, |el, (text, color)| {
        el.child(note("files-search-note", text, color))
    })
    .when(v.found.cut, |el| {
        el.child(note(
            "files-search-cut",
            "Search results limited to 100 names and 500 lines".into(),
            t.dim,
        ))
    })
    .into_any_element()
}

fn group(t: &Theme, title: &'static str) -> AnyElement {
    div()
        .px(px(10.))
        .pt(px(6.))
        .pb(px(2.))
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.dim)
        .child(title)
        .into_any_element()
}

fn menu_layer(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> Option<AnyElement> {
    let m = v.menu.as_ref()?;
    let items: Vec<MenuItem> = actions::menu_items(&m.target)
        .into_iter()
        .map(|a| {
            let item = MenuItem::new(a.label()).icon(action_icon(a));
            let item = match a {
                FileAction::Gitignore => item.shortcut("Shared .gitignore"),
                FileAction::Exclude => item.shortcut("Local .git/info/exclude"),
                _ => item,
            };
            let item = if a == FileAction::Delete {
                item.danger()
            } else {
                item
            };
            if matches!(
                a,
                FileAction::NewFile | FileAction::Reveal | FileAction::Edit
            ) {
                item.separated()
            } else {
                item
            }
        })
        .collect();
    let this = cx.entity().downgrade();
    let on_pick: nixa_ui::menu::OnPick = Rc::new(move |index, window, app| {
        this.update(app, |v, cx| super::pick(v, index, window, cx))
            .ok();
    });
    let title = SharedString::from(file_name(&m.target.path).to_string());
    let panel = menu(t, "files-menu", Some(title), &items, m.hi, on_pick)
        .w(px(280.))
        .debug_selector(|| "files-menu".into())
        .on_mouse_down_out(cx.listener(|v, _, _, cx| super::close_menu(v, cx)));
    Some(float_at(m.at, Corner::TopLeft, panel).into_any_element())
}

fn action_icon(a: FileAction) -> Icon {
    match a {
        FileAction::Run => Icon::Play,
        FileAction::Gitignore | FileAction::Exclude => Icon::GitBranch,
        FileAction::Edit => Icon::SquarePen,
        FileAction::Terminal => Icon::Terminal,
        FileAction::NewFile => Icon::Plus,
        FileAction::NewFolder => Icon::FolderPlus,
        FileAction::Rename => Icon::SquarePen,
        FileAction::Delete => Icon::Trash,
        FileAction::Reveal => Icon::Search,
        FileAction::Open => Icon::Monitor,
        FileAction::Copy => Icon::Copy,
    }
}

fn prompt_layer(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> Option<AnyElement> {
    let p = v.prompt.as_ref()?;
    let title = match p.action {
        FileAction::NewFile => "New file",
        FileAction::NewFolder => "New folder",
        _ => "Rename",
    };
    let body = div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(div().text_size(px(11.5)).text_color(t.dim).child("Name"))
        .child(
            div()
                .id("files-prompt-name")
                .debug_selector(|| "files-prompt-name".into())
                .w_full()
                .flex()
                .flex_col()
                .child(p.input.clone()),
        )
        .child(div().text_size(px(11.)).text_color(t.dim).child("Enter to save · Esc to cancel"))
        .when_some(p.error.clone(), |el, e| {
            el.child(
                div()
                    .id("files-prompt-error")
                    .debug_selector(|| "files-prompt-error".into())
                    .text_size(px(11.5))
                    .text_color(t.red)
                    .child(SharedString::from(e)),
            )
        });
    let footer = div()
        .flex()
        .gap(px(8.))
        .child(
            button(t, "files-prompt-cancel", "Cancel", ButtonKind::Secondary)
                .debug_selector(|| "files-prompt-cancel".into())
                .on_click(cx.listener(|v, _, _, cx| super::close_dialogs(v, cx))),
        )
        .child(
            button(t, "files-prompt-save", "Save", ButtonKind::Primary)
                .debug_selector(|| "files-prompt-save".into())
                .on_click(cx.listener(|v, _, _, cx| {
                    let text = v
                        .prompt
                        .as_ref()
                        .map(|p| p.input.read(cx).value().to_string())
                        .unwrap_or_default();
                    super::submit(v, text, cx);
                })),
        );
    Some(modal(
        t,
        "files-prompt",
        title,
        body.into_any_element(),
        footer.into_any_element(),
        cx,
    ))
}

fn confirm_layer(v: &FilesView, t: &Theme, cx: &mut Context<FilesView>) -> Option<AnyElement> {
    let c = v.confirm.as_ref()?;
    let what = if c.target.dir {
        "this folder and everything in it"
    } else {
        "this file"
    };
    let body = div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .text_color(t.muted)
                .child(SharedString::from(format!("Move {what} to the Trash?"))),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.dim)
                .child(SharedString::from(c.target.full())),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.dim)
                .child("Enter to delete · Esc to cancel"),
        )
        .when_some(c.error.clone(), |el, e| {
            el.child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.red)
                    .child(SharedString::from(e)),
            )
        });
    let footer = div()
        .flex()
        .gap(px(8.))
        .child(
            button(t, "files-delete-cancel", "Cancel", ButtonKind::Secondary)
                .debug_selector(|| "files-delete-cancel".into())
                .on_click(cx.listener(|v, _, _, cx| super::close_dialogs(v, cx))),
        )
        .child(
            button(t, "files-delete-yes", "Delete", ButtonKind::Danger)
                .debug_selector(|| "files-delete-yes".into())
                .on_click(cx.listener(|v, _, _, cx| super::confirm_delete(v, cx))),
        );
    Some(modal(
        t,
        "files-confirm",
        "Delete?",
        body.into_any_element(),
        footer.into_any_element(),
        cx,
    ))
}

fn modal(
    t: &Theme,
    id: &'static str,
    title: &'static str,
    body: AnyElement,
    footer: AnyElement,
    cx: &mut Context<FilesView>,
) -> AnyElement {
    let close = cx.entity().downgrade();
    let dismiss = close.clone();
    let card = dialog(t, id, title, 340., body, Some(footer), move |_, app| {
        close.update(app, super::close_dialogs).ok();
    })
    .debug_selector(move || id.to_string());
    overlay(
        t,
        SharedString::from(format!("{id}-scrim")),
        card,
        move |_, app| {
            dismiss.update(app, super::close_dialogs).ok();
        },
    )
    .into_any_element()
}

fn on_key(v: &mut FilesView, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<FilesView>) {
    let key = e.keystroke.key.as_str();
    if v.confirm.is_some() {
        match key {
            "enter" => super::confirm_delete(v, cx),
            "escape" => super::close_dialogs(v, cx),
            _ => return,
        }
        cx.stop_propagation();
        return;
    }
    let Some(m) = v.menu.as_ref() else { return };
    let items: Vec<MenuItem> = actions::menu_items(&m.target)
        .into_iter()
        .map(|a| MenuItem::new(a.label()))
        .collect();
    match menu_key(&items, m.hi, key) {
        MenuKey::Move(i) => {
            if let Some(m) = v.menu.as_mut() {
                m.hi = Some(i);
            }
            cx.notify();
        }
        MenuKey::Choose(i) => super::pick(v, i, window, cx),
        MenuKey::Close => super::close_menu(v, cx),
        MenuKey::Ignore => return,
    }
    cx.stop_propagation();
}

impl Render for FilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let searching = !self.query.trim().is_empty();
        let body = if searching {
            search_body(self, &t, cx)
        } else {
            tree_body(self, &t, cx)
        };
        div()
            .id("files")
            .key_context("NixaFiles")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(on_key))
            .relative()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(13.))
            .child(header(self, &t, cx))
            .child(search_bar(self, &t, cx))
            .child(body)
            .children(menu_layer(self, &t, cx))
            .children(prompt_layer(self, &t, cx))
            .children(confirm_layer(self, &t, cx))
    }
}
