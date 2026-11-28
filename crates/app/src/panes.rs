mod drag;
mod links;

use crate::layout::{self, Axis, Node};
use crate::spaces;
use crate::tabs::{self, Tab};
use crate::terms;
use crate::workspace::{EngineState, Workspace};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, DragMoveEvent, Empty, FontWeight, Hsla, MouseButton, SharedString,
    Window, div, px,
};
use nixa_protocol::{TermId, TermInfo};
use nixa_ui::button::{ButtonKind, button, icon_label_button};
use nixa_ui::chip::chip;
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::metrics::{PANE_GAP, PANE_HEADER_HEIGHT, PANE_RADIUS};
use nixa_ui::popover::tooltip;
use nixa_ui::tab::pane as pane_frame;
use nixa_ui::{Icon, Theme, icon, theme};

pub const ASSISTANT: &str = "Nixa";
const EDGE_GROUP: &str = "pane-edge";

#[derive(Debug, Clone, PartialEq)]
pub struct PaneDrag {
    pub tab: String,
    pub path: Vec<usize>,
    pub edge: usize,
}

struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Lone,
    Split,
    Zoomed,
}

fn path_key(path: &[usize]) -> String {
    path.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

pub fn edge_id(tab: &str, path: &[usize], edge: usize) -> SharedString {
    format!("pane-edge-{tab}-{}-{edge}", path_key(path)).into()
}

pub fn number_tip(number: u32, id: &TermId, assistant: &str) -> String {
    format!("Terminal {number} ({id}). Say \"terminal {number}\" to {assistant}.")
}

fn clean(text: &str) -> String {
    text.trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}

fn path_like(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('~')
        || text.starts_with('/')
        || (bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\')
}

pub fn program_name(term: &TermInfo) -> String {
    let name = terms::program(&term.cmd);
    if name.is_empty() {
        "shell".to_string()
    } else {
        name
    }
}

pub fn auto_title(term: &TermInfo) -> String {
    let prog = program_name(term);
    let own = term.name.trim();
    if !own.is_empty() && own != prog && !own.chars().all(|c| c.is_ascii_digit()) {
        return own.to_string();
    }
    if terms::is_shell(&term.cmd) && term.status.is_none() {
        return String::new();
    }
    let raw = term.title.as_deref().unwrap_or_default().trim();
    let live = clean(raw);
    let telling = !live.is_empty()
        && live != prog
        && !path_like(raw)
        && !path_like(&live)
        && !live.contains('@')
        && live.to_lowercase() != prog.to_lowercase()
        && !live.chars().all(|c| c.is_ascii_digit());
    if telling { live } else { String::new() }
}

pub fn link_path(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    let path = match text.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", crate::spaces::home()),
        None => text.to_string(),
    };
    (path.starts_with('/') && std::path::Path::new(&path).exists()).then_some(path)
}

fn term_view(ws: &Workspace, t: &Theme, id: &TermId, term: Option<&TermInfo>) -> AnyElement {
    match ws.views.get(id) {
        Some(view) => view.clone().into_any_element(),
        None => placeholder(t, id, term),
    }
}

fn placeholder(t: &Theme, id: &TermId, term: Option<&TermInfo>) -> AnyElement {
    let line = match term {
        Some(term) if term.running => {
            format!("Terminal {id} is running {}", terms::term_name(term))
        }
        Some(term) => format!(
            "Terminal {id} ended{}",
            term.exit_code
                .map(|c| format!(" with code {c}"))
                .unwrap_or_default()
        ),
        None => format!("Terminal {id}"),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .bg(t.background)
        .text_color(t.dim)
        .child(
            icon(Icon::Terminal)
                .size(px(22.))
                .text_color(t.border_strong),
        )
        .child(div().text_size(px(12.5)).child(SharedString::from(line)))
        .into_any_element()
}

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let t = theme(cx).clone();
    let Some(tab) = ws.active_tab().cloned() else {
        return empty(ws, &t, cx);
    };
    if let Some(id) = tabs::zoomed(&tab).cloned() {
        return pane(ws, &t, &tab, &id, Mode::Zoomed, cx);
    }
    let mode = if matches!(tab.root, Node::Leaf(_)) {
        Mode::Lone
    } else {
        Mode::Split
    };
    node(ws, &t, &tab, &tab.root, Vec::new(), mode, cx)
}

pub fn zoom(ws: &mut Workspace, tab_id: &str, id: &TermId, cx: &mut Context<Workspace>) {
    if tabs::toggle_zoom(&mut ws.tabs, tab_id, id) {
        cx.notify();
    }
}

fn save(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if ws.engine != EngineState::Up {
        return;
    }
    if let Err(e) = tabs::persist(&mut ws.tabs, &ws.tabs_path) {
        ws.notice = Some(format!("Cannot save the pane layout: {e}"));
        cx.notify();
    }
}

fn drag_edge(
    ws: &mut Workspace,
    event: &DragMoveEvent<PaneDrag>,
    tab: &str,
    path: &[usize],
    axis: Axis,
    cx: &mut Context<Workspace>,
) {
    let drag = event.drag(cx).clone();
    if drag.tab != tab || drag.path != path {
        return;
    }
    let mouse = event.event.position;
    let (at, length) = match axis {
        Axis::Row => (mouse.x - event.bounds.left(), event.bounds.size.width),
        Axis::Column => (mouse.y - event.bounds.top(), event.bounds.size.height),
    };
    if tabs::resize(
        &mut ws.tabs,
        tab,
        path,
        drag.edge,
        f32::from(at),
        f32::from(length),
        PANE_GAP,
    ) {
        cx.notify();
    }
}

fn empty(ws: &Workspace, t: &Theme, cx: &mut Context<Workspace>) -> AnyElement {
    let folder = ws
        .active_space()
        .map(|s| spaces::short_folder(&s.folder, &spaces::home()))
        .unwrap_or_default();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .rounded(px(PANE_RADIUS))
        .border_1()
        .border_color(t.border)
        .bg(t.background)
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|ws, e: &gpui::MouseDownEvent, window, cx| {
                ws.open_ctx(
                    crate::screens::ctxmenu::Target::Default,
                    e.position,
                    window,
                    cx,
                )
            }),
        )
        .child(
            icon(Icon::Terminal)
                .size(px(26.))
                .text_color(t.border_strong),
        )
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.muted)
                .child("No tabs in this space"),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(t.dim)
                .child(SharedString::from(folder)),
        )
        .child(
            icon_label_button(
                t,
                "empty-new-tab",
                Icon::Plus,
                "New tab",
                ButtonKind::Secondary,
            )
            .mt(px(4.))
            .on_click(cx.listener(|ws, _, _, cx| ws.new_tab(Vec::new(), None, cx)))
            .child(div().text_color(t.dim).child("⌘T")),
        )
        .when(setup_left(cx), |el| el.child(setup_link(t)))
        .into_any_element()
}

fn setup_left(cx: &gpui::App) -> bool {
    crate::screens::first_run::host::view(cx).is_some_and(|v| !v.read(cx).done)
}

fn setup_link(t: &Theme) -> AnyElement {
    button(t, "empty-setup", "Finish setting up Nixa", ButtonKind::Ghost)
        .debug_selector(|| "empty-setup".into())
        .on_click(|_, _, cx| crate::screens::first_run::host::run(crate::screens::first_run::Action::Open, cx))
        .into_any_element()
}

fn edge(t: &Theme, tab: &str, path: &[usize], index: usize, axis: Axis) -> AnyElement {
    let active = t.accent;
    let bar = match axis {
        Axis::Row => div().w(px(2.)).h_full(),
        Axis::Column => div().h(px(2.)).w_full(),
    };
    let key = edge_id(tab, path, index);
    let handle = div()
        .id(key.clone())
        .debug_selector(|| key.to_string())
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .group(EDGE_GROUP);
    let handle = match axis {
        Axis::Row => handle.w(px(PANE_GAP)).h_full().cursor_col_resize(),
        Axis::Column => handle.h(px(PANE_GAP)).w_full().cursor_row_resize(),
    };
    handle
        .child(
            bar.rounded(px(1.))
                .group_hover(EDGE_GROUP, move |s| s.bg(active)),
        )
        .on_drag(
            PaneDrag {
                tab: tab.to_string(),
                path: path.to_vec(),
                edge: index,
            },
            |_, _, _, cx| cx.new(|_| DragGhost),
        )
        .into_any_element()
}

fn node(
    ws: &Workspace,
    t: &Theme,
    tab: &Tab,
    n: &Node,
    path: Vec<usize>,
    mode: Mode,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let Node::Split {
        axis,
        children,
        sizes,
    } = n
    else {
        let Node::Leaf(id) = n else {
            return div().into_any_element();
        };
        return pane(ws, t, tab, id, mode, cx);
    };
    let axis = *axis;
    let shares = layout::shares(sizes, children.len());
    let mut items = Vec::with_capacity(children.len() * 2);
    for (i, (child, share)) in children.iter().zip(shares).enumerate() {
        if i > 0 {
            items.push(edge(t, &tab.id, &path, i - 1, axis));
        }
        let mut child_path = path.clone();
        child_path.push(i);
        let mut cell = div().flex_1().min_w_0().min_h_0();
        cell.style().flex_grow = Some(share);
        items.push(
            cell.child(node(ws, t, tab, child, child_path, Mode::Split, cx))
                .into_any_element(),
        );
    }
    let tab_id = tab.id.clone();
    let here = path.clone();
    let key = format!("pane-split-{}-{}", tab.id, path_key(&path));
    div()
        .id(SharedString::from(key.clone()))
        .debug_selector(|| key)
        .size_full()
        .flex()
        .map(|el| {
            if axis == Axis::Row {
                el.flex_row()
            } else {
                el.flex_col()
            }
        })
        .on_drag_move(
            cx.listener(move |ws, event: &DragMoveEvent<PaneDrag>, _, cx| {
                drag_edge(ws, event, &tab_id, &here, axis, cx)
            }),
        )
        .on_drop(cx.listener(|ws, _: &PaneDrag, _, cx| save(ws, cx)))
        .children(items)
        .into_any_element()
}

fn split_glyph(axis: Axis, color: Hsla) -> Div {
    let line = match axis {
        Axis::Row => div().absolute().top_0().bottom_0().left(px(4.5)).w(px(1.)),
        Axis::Column => div().absolute().left_0().right_0().top(px(4.5)).h(px(1.)),
    };
    div()
        .relative()
        .flex_none()
        .size(px(12.))
        .rounded(px(2.))
        .border_1()
        .border_color(color)
        .child(line.bg(color))
}

fn split_button(t: &Theme, id: SharedString, axis: Axis) -> gpui::Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id)
        .flex_none()
        .size(px(IconSize::Small.box_size()))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(split_glyph(axis, t.dim))
}

fn pane(
    ws: &Workspace,
    t: &Theme,
    tab: &Tab,
    id: &TermId,
    mode: Mode,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let term = ws.terms.get(id);
    let focused = tab.focus == *id && mode == Mode::Split;
    let zoomed = mode == Mode::Zoomed;
    let hover_id = id.clone();
    let show_actions = ws.hovered_pane_header.as_ref() == Some(id);
    let name = term
        .map(terms::term_name)
        .unwrap_or_else(|| "shell".to_string());
    let auto = term.map(auto_title).filter(|title| title != &name).unwrap_or_default();
    let cmd = term.map(|t| t.cmd.clone()).unwrap_or_default();
    let folder = term
        .map(|t| spaces::short_folder(&t.cwd, &spaces::home()))
        .unwrap_or_default();
    let number = ws.tabs.numbers.get(id).copied();
    let tab_id = tab.id.clone();
    let focus_id = id.clone();
    let close_id = id.clone();
    let mid_close_id = id.clone();
    let zoom_tab = tab.id.clone();
    let zoom_id = id.clone();
    let menu_tab = tab.id.clone();
    let menu_id = id.clone();
    let aim_tab = tab.id.clone();
    let aim_id = id.clone();
    let drop_tab = tab.id.clone();
    let drop_id = id.clone();
    let move_drag = drag::PaneMove {
        tab: tab.id.clone(),
        term: id.clone(),
        label: name.clone(),
    };
    let aimed = drag::overlay(t, &tab.id, id, cx);
    let split = |axis, suffix, label| {
        let tab_id = tab.id.clone();
        let target = id.clone();
        split_button(t, format!("ph-{suffix}-{id}").into(), axis)
            .tooltip(tooltip(label))
            .on_click(cx.listener(move |ws, _, _, cx| {
                cx.stop_propagation();
                ws.split_pane(&tab_id, &target, axis, cx);
            }))
    };
    let actions = show_actions.then(|| {
        div().absolute().right_0().top_0().bottom_0()
            .flex().items_center().gap(px(4.)).px(px(6.)).bg(t.chrome)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(split(Axis::Row, "r", "Split right"))
            .child(split(Axis::Column, "d", "Split down"))
            .child(icon_button(
                t, SharedString::from(format!("ph-z-{id}")),
                if zoomed { Icon::Copy } else { Icon::Square }, IconSize::Small, zoomed,
            )
            .debug_selector(|| format!("ph-z-{id}"))
            .tooltip(tooltip(if zoomed { "Unzoom pane" } else { "Zoom pane" }))
            .on_click(cx.listener(move |ws, _, _, cx| {
                cx.stop_propagation();
                zoom(ws, &zoom_tab, &zoom_id, cx);
            })))
            .child(icon_button(t, SharedString::from(format!("ph-x-{id}")), Icon::X, IconSize::Small, false)
                .tooltip(tooltip("Close pane"))
                .on_click(cx.listener(move |ws, _, _, cx| {
                    cx.stop_propagation();
                    ws.close_term(close_id.clone(), cx);
                })))
    });
    let header = div()
        .id(SharedString::from(format!("pane-head-{id}")))
        .debug_selector(|| format!("pane-head-{id}"))
        .h(px(PANE_HEADER_HEIGHT))
        .relative()
        .on_hover(cx.listener(move |ws, hovered: &bool, _, cx| {
            if *hovered {
                ws.hovered_pane_header = Some(hover_id.clone());
            } else if ws.hovered_pane_header.as_ref() == Some(&hover_id) {
                ws.hovered_pane_header = None;
            }
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |ws, _, _, cx| {
                cx.stop_propagation();
                ws.close_term(mid_close_id.clone(), cx);
            }),
        )
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .border_b_1()
        .border_color(t.border)
        .text_size(px(12.))
        .when(mode == Mode::Split, move |el| el.on_drag(move_drag, drag::start))
        .child(terms::program_badge(t, &cmd, 14.))
        .child(
            div()
                .flex_none()
                .max_w(px(260.))
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.bright)
                .child(SharedString::from(name)),
        )
        .when_some(number, |el, n| {
            el.child(
                chip(format!("#{n}"), t.magenta)
                    .id(SharedString::from(format!("pane-number-{id}")))
                    .debug_selector(|| format!("pane-number-{id}"))
                    .tooltip(tooltip(number_tip(n, id, ASSISTANT))),
            )
        })
        .when(!auto.is_empty(), |el| {
            let tip = if folder.is_empty() {
                auto.clone()
            } else {
                format!("{auto} · {folder}")
            };
            el.child(
                div()
                    .id(SharedString::from(format!("pane-title-{id}")))
                    .flex_shrink()
                    .min_w_0()
                    .max_w(px(360.))
                    .truncate()
                    .text_color(t.foreground)
                    .tooltip(tooltip(tip))
                    .child(SharedString::from(auto.clone())),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if auto.is_empty() { t.dim } else { t.muted })
                .when(!auto.is_empty(), |el| el.text_size(px(11.)))
                .child(SharedString::from(folder.clone())),
        )
        .child(terms::status_mark(
            t,
            SharedString::from(format!("pane-status-{id}")),
            term,
        ))
        .children(actions);
    pane_frame(t, focused)
        .id(SharedString::from(format!("pane-{id}")))
        .debug_selector(|| format!("pane-{id}"))
        .relative()
        .size_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |ws, e: &gpui::MouseDownEvent, window, cx| {
                if e.modifiers.platform && links::click(ws, &focus_id, e.position, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                ws.focus_pane(&tab_id, &focus_id, cx)
            }),
        )
        .on_drag_move(
            cx.listener(move |_, event: &DragMoveEvent<drag::PaneMove>, _, cx| {
                drag::aim(event, &aim_tab, &aim_id, cx)
            }),
        )
        .on_drop(cx.listener(move |ws, moving: &drag::PaneMove, _, cx| {
            drag::drop_on(ws, moving, &drop_tab, &drop_id, cx)
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |ws, e: &gpui::MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let path = ws
                    .views
                    .get(&menu_id)
                    .and_then(|v| v.read(cx).selected_text())
                    .and_then(|text| link_path(&text));
                let target = match path {
                    Some(path) => crate::screens::ctxmenu::Target::Link { path },
                    None => crate::screens::ctxmenu::Target::Pane {
                        tab: menu_tab.clone(),
                        terminal: menu_id.clone(),
                    },
                };
                ws.open_ctx(target, e.position, window, cx);
            }),
        )
        .child(header)
        .child(div().flex_1().min_h_0().child(term_view(ws, t, id, term)))
        .children(aimed)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn a_selected_path_is_a_link() {
        assert_eq!(link_path("  /tmp "), Some("/tmp".to_string()));
        assert_eq!(link_path("/no/such/nixa/path"), None);
        assert_eq!(link_path("hello"), None);
        assert_eq!(link_path("/tmp\n/usr"), None);
    }

    fn info(cmd: &[&str], name: &str, title: Option<&str>) -> TermInfo {
        TermInfo {
            id: TermId::from_number(1),
            name: name.to_string(),
            labels: BTreeMap::new(),
            parent: None,
            children: Vec::new(),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            cwd: "/".into(),
            pid: None,
            running: true,
            exit_code: None,
            status: None,
            status_note: None,
            keep: false,
            draft: false,
            title: title.map(str::to_string),
            rows: 24,
            cols: 80,
            created_ms: 0,
            seq: 0,
        }
    }

    #[test]
    fn number_tip_names_the_words_to_say() {
        assert_eq!(
            number_tip(3, &TermId::from_number(12), ASSISTANT),
            "Terminal 3 (mc). Say \"terminal 3\" to Nixa."
        );
        assert_eq!(edge_id("t4", &[1, 0], 2).as_ref(), "pane-edge-t4-1.0-2");
        assert_eq!(edge_id("t4", &[], 0).as_ref(), "pane-edge-t4--0");
    }

    #[test]
    fn auto_title_shows_a_telling_title_for_agents() {
        assert_eq!(
            auto_title(&info(&["claude"], "", Some("✳ Fix the login bug"))),
            "Fix the login bug"
        );
        assert_eq!(auto_title(&info(&["claude"], "", Some("claude"))), "");
        assert_eq!(auto_title(&info(&["claude"], "", Some("Claude"))), "");
        assert_eq!(
            auto_title(&info(&["claude"], "", Some("~/Projects/nixa"))),
            ""
        );
        assert_eq!(auto_title(&info(&["claude"], "", Some("someone@mac"))), "");
        assert_eq!(auto_title(&info(&["claude"], "", Some("C:\\work"))), "");
        assert_eq!(auto_title(&info(&["claude"], "", Some("42"))), "");
        assert_eq!(auto_title(&info(&["claude"], "", None)), "");
    }

    #[test]
    fn auto_title_for_shells_needs_a_status_or_a_name() {
        assert_eq!(auto_title(&info(&[], "", Some("vim notes.md"))), "");
        let mut working = info(&[], "", Some("cargo test"));
        working.status = Some(nixa_protocol::Status::Working);
        assert_eq!(auto_title(&working), "cargo test");
        assert_eq!(auto_title(&info(&[], "build box", None)), "build box");
        assert_eq!(
            auto_title(&info(&["claude"], "reviewer", Some("x"))),
            "reviewer"
        );
        assert_eq!(
            auto_title(&info(&["claude"], "claude", Some("Plan"))),
            "Plan"
        );
        assert_eq!(auto_title(&info(&["claude"], "7", None)), "");
        assert_eq!(program_name(&info(&[], "", None)), "shell");
        assert_eq!(program_name(&info(&["/usr/bin/htop"], "", None)), "htop");
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::layout::row;
    use gpui::{Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, point, size};
    use nixa_protocol::paths;
    use std::path::PathBuf;
    use std::sync::Once;

    fn t(n: u64) -> TermId {
        TermId::from_number(n)
    }

    fn data_dir() -> PathBuf {
        PathBuf::from(format!("/tmp/mo-w210-{}", std::process::id()))
    }

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
        root: Node,
    ) -> (Entity<Workspace>, &'a mut VisualTestContext) {
        static ENV: Once = Once::new();
        ENV.call_once(|| unsafe {
            std::env::set_var(paths::ENV_DATA_DIR, data_dir());
            std::env::set_var(paths::ENV_ENGINE_SOCKET, data_dir().join("e.sock"));
        });
        cx.update(nixa_ui::init);
        let (ws, cx) = cx.add_window_view(Workspace::new);
        cx.simulate_resize(size(px(1280.), px(800.)));
        ws.update(cx, |ws, cx| {
            ws.engine = EngineState::Up;
            ws.tabs_path = data_dir().join(format!("{name}-tabs.json"));
            if ws.spaces.spaces.is_empty() {
                crate::spaces::add(&mut ws.spaces, &crate::spaces::home());
            }
            let space = ws.active_space().map(|s| s.id.clone()).expect("a space");
            let focus = layout::leaves(&root)[0].clone();
            ws.tabs.list.retain(|tab| tab.id != "t1");
            ws.tabs.list.push(Tab {
                id: "t1".into(),
                space: space.clone(),
                root,
                focus,
                zoom: None,
            });
            ws.tabs.active.insert(space, "t1".into());
            ws.tabs.numbers.insert(t(1), 1);
            ws.tabs.numbers.insert(t(2), 2);
            cx.notify();
        });
        cx.run_until_parked();
        (ws, cx)
    }

    fn bounds(cx: &mut VisualTestContext, key: &'static str) -> Bounds<Pixels> {
        cx.debug_bounds(key)
            .unwrap_or_else(|| panic!("{key} is not drawn"))
    }

    fn root(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Node {
        ws.read_with(cx, |ws, _| {
            tabs::find(&ws.tabs, "t1").expect("tab t1").root.clone()
        })
    }

    #[gpui::test]
    fn right_click_on_a_pane_opens_its_menu(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "ctx", row(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]));
        let pane = bounds(cx, "pane-m2");
        cx.simulate_mouse_down(pane.center(), MouseButton::Right, Modifiers::none());
        cx.run_until_parked();
        let target = ws.read_with(cx, |ws, cx| {
            ws.ctx.as_ref().map(|c| c.view().read(cx).target.clone())
        });
        assert_eq!(
            target,
            Some(crate::screens::ctxmenu::Target::Pane {
                tab: "t1".into(),
                terminal: t(2),
            })
        );
    }

    #[gpui::test]
    fn one_overlay_at_a_time(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "overlays", row(vec![Node::Leaf(t(1))]));
        cx.update(|window, cx| {
            ws.update(cx, |ws, cx| {
                crate::screens::tabbar::run(ws, crate::screens::tabbar::Action::NewMenu, window, cx)
            })
        });
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, cx| ws.tab_bar.read(cx).menu.is_some()));
        cx.update(|window, cx| ws.update(cx, |ws, cx| ws.open_palette(window, cx)));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.palette.is_some());
            assert!(
                ws.tab_bar.read(cx).menu.is_none(),
                "the palette must not open under the + menu"
            );
        });
        cx.update(|window, cx| ws.update(cx, |ws, cx| ws.open_picker(window, cx)));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.picker.is_some());
            assert!(ws.palette.is_none());
        });
        cx.update(|window, cx| ws.update(cx, |ws, cx| ws.open_favourites(window, cx)));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.favourites.is_some());
            assert!(ws.picker.is_none());
        });
    }

    #[gpui::test]
    fn the_command_list_has_the_wave_three_screens(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "cmds", row(vec![Node::Leaf(t(1))]));
        let labels: Vec<String> = ws.read_with(cx, |ws, _| {
            crate::commands::items(ws)
                .into_iter()
                .map(|i| i.label)
                .collect()
        });
        for want in [
            "Go to a tab, space or agent",
            "Favourites of this project",
            "Show the actions menu",
            "Next theme",
            "Agents: Next agent",
            "Space: New space",
            "Open the Nixa screen",
        ] {
            assert!(
                labels.iter().any(|l| l == want),
                "{want} is missing from {labels:?}"
            );
        }
    }

    #[gpui::test]
    fn dragging_the_edge_resizes_and_saves(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "drag", row(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]));
        let split = bounds(cx, "pane-split-t1-");
        let edge = bounds(cx, "pane-edge-t1--0");
        let start = edge.center();
        let target = point(split.left() + split.size.width * 0.3, start.y);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(
            point(start.x - px(30.), start.y),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let now = root(&ws, cx);
        let Node::Split { sizes, .. } = &now else {
            panic!("not a split: {now:?}");
        };
        let width = f32::from(split.size.width);
        let want = (width * 0.3 - PANE_GAP / 2.0) / (width - PANE_GAP);
        assert!(
            (sizes[0] - want).abs() < 0.01,
            "sizes {sizes:?}, want {want}"
        );
        let left = bounds(cx, "pane-m1");
        let right = bounds(cx, "pane-m2");
        assert!((f32::from(left.size.width) - want * (width - PANE_GAP)).abs() < 2.0);
        assert!(left.size.width < right.size.width);
        let (saved, path) = ws.read_with(cx, |ws, _| {
            (ws.tabs.saved.get("t1").cloned(), ws.tabs_path.clone())
        });
        assert_eq!(saved.as_ref(), Some(&now));
        assert_eq!(tabs::load_layouts(&path).get("t1"), Some(&now));
    }

    #[gpui::test]
    fn dragging_a_nested_edge_moves_only_that_split(cx: &mut TestAppContext) {
        let start_root = row(vec![
            Node::Leaf(t(1)),
            layout::column(vec![Node::Leaf(t(2)), Node::Leaf(t(3))]),
        ]);
        let (ws, cx) = setup(cx, "nested", start_root);
        let inner = bounds(cx, "pane-split-t1-1");
        let edge = bounds(cx, "pane-edge-t1-1-0");
        let start = edge.center();
        let target = point(start.x, inner.top() + inner.size.height * 0.75);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(
            point(start.x, start.y + px(30.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let now = root(&ws, cx);
        let Node::Split {
            sizes: outer,
            children,
            ..
        } = &now
        else {
            panic!("not a split: {now:?}");
        };
        assert!(outer.is_empty(), "outer sizes {outer:?}");
        let Node::Split { sizes, .. } = &children[1] else {
            panic!("inner is not a split");
        };
        let height = f32::from(inner.size.height);
        let want = (height * 0.75 - PANE_GAP / 2.0) / (height - PANE_GAP);
        assert!(
            (sizes[0] - want).abs() < 0.01,
            "sizes {sizes:?}, want {want}"
        );
        assert!(bounds(cx, "pane-m2").size.height > bounds(cx, "pane-m3").size.height);
    }

    #[gpui::test]
    fn dragging_a_header_moves_the_pane(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "move", row(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]));
        let head = bounds(cx, "pane-head-m2");
        let left = bounds(cx, "pane-m1");
        let start = head.center();
        let target = point(left.left() + px(10.), left.center().y);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(
            point(start.x - px(30.), start.y),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(layout::leaves(&root(&ws, cx)), vec![t(2), t(1)]);
        let focus = ws.read_with(cx, |ws, _| {
            tabs::find(&ws.tabs, "t1").map(|tab| tab.focus.clone())
        });
        assert_eq!(focus, Some(t(2)));
    }

    #[gpui::test]
    fn zoom_button_zooms_and_unzooms(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "zoom", row(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]));
        let full = bounds(cx, "pane-split-t1-");
        hover_header(cx, "pane-head-m2");
        let button = bounds(cx, "ph-z-m2");
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked();
        let zoom = ws.read_with(cx, |ws, _| {
            let tab = tabs::find(&ws.tabs, "t1").expect("tab t1");
            (tabs::zoomed(tab).cloned(), tab.focus.clone())
        });
        assert_eq!(zoom, (Some(t(2)), t(2)));
        let big = bounds(cx, "pane-m2");
        assert!(big.size.width >= full.size.width - px(1.));
        assert!((big.left() - full.left()).abs() < px(1.));
        hover_header(cx, "pane-head-m2");
        let button = bounds(cx, "ph-z-m2");
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked();
        let zoom = ws.read_with(cx, |ws, _| {
            tabs::find(&ws.tabs, "t1").and_then(tabs::zoomed).cloned()
        });
        assert_eq!(zoom, None);
        let left = bounds(cx, "pane-m1");
        let right = bounds(cx, "pane-m2");
        assert!(left.size.width < full.size.width / 2.);
        assert!(right.size.width < full.size.width / 2.);
        assert!(left.right() < right.left());
    }

    #[gpui::test]
    fn a_split_while_zoomed_shows_every_pane(cx: &mut TestAppContext) {
        let (ws, cx) = setup(
            cx,
            "zoom-split",
            row(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]),
        );
        hover_header(cx, "pane-head-m1");
        let button = bounds(cx, "ph-z-m1");
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked();
        let full = bounds(cx, "pane-split-t1-");
        let zoomed = bounds(cx, "pane-m1");
        assert!(zoomed.size.width >= full.size.width - px(1.));
        ws.update(cx, |ws, cx| {
            let tab = tabs::find_mut(&mut ws.tabs, "t1").expect("tab t1");
            tab.root = layout::split(tab.root.clone(), &t(1), &t(3), Axis::Column);
            tab.focus = t(3);
            cx.notify();
        });
        cx.run_until_parked();
        let one = bounds(cx, "pane-m1");
        let two = bounds(cx, "pane-m2");
        let three = bounds(cx, "pane-m3");
        assert!(one.size.width < full.size.width / 2.);
        assert!(two.left() > one.right());
        assert!(three.top() > one.bottom());
        assert!((three.left() - one.left()).abs() < px(1.));
    }

    fn hover_header(cx: &mut VisualTestContext, header: &'static str) {
        let point = bounds(cx, header).center();
        cx.simulate_mouse_move(point, None, Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn lone_pane_shows_zoom_only_on_hover(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx, "lone", Node::Leaf(t(1)));
        assert!(cx.debug_bounds("pane-number-m1").is_some());
        assert!(cx.debug_bounds("ph-z-m1").is_none());
        hover_header(cx, "pane-head-m1");
        let button = bounds(cx, "ph-z-m1");
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| tabs::find(&ws.tabs, "t1").and_then(tabs::zoomed).is_some()));
    }
}
