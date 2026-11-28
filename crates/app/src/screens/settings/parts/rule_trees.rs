use super::memory::doc::call;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, SharedString, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const WATCH_CHOICES: [u64; 7] = [0, 2, 5, 10, 15, 30, 60];
pub const JSON_BIN: &str = "nixa-json";
pub const ENV_JSON_BIN: &str = "NIXA_JSON_BIN";

#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub name: String,
    pub title: String,
    pub when: String,
    pub boxes: u64,
    pub broken: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSeat {
    pub id: String,
    pub space_name: String,
    pub name: String,
    pub terminal: Option<String>,
    pub state: String,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepAsk {
    pub id: String,
    pub space_name: String,
    pub state: String,
    pub question: String,
    pub pick: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleTreesEvent {
    EditJson { name: String, argv: Vec<String> },
}

pub struct RuleTreesView {
    pub addr: String,
    pub trees: Option<Vec<Tree>>,
    pub error: Option<String>,
    pub watch: Option<u64>,
    pub watch_error: Option<String>,
    pub deep_auto_start: Option<bool>,
    pub deep_error: Option<String>,
    pub deep_seats: Option<Vec<DeepSeat>>,
    pub deep_open: Vec<DeepAsk>,
    pub deep_recent: Vec<DeepAsk>,
    pub deep_status_error: Option<String>,
    pub data_dir: Option<String>,
    pub name: String,
}

impl EventEmitter<RuleTreesEvent> for RuleTreesView {}

pub fn good_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= 60
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn parse_trees(value: &Value) -> Vec<Tree> {
    let text = |t: &Value, k: &str| {
        t.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    value
        .get("trees")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|t| t.get("name").and_then(Value::as_str).is_some_and(good_name))
                .map(|t| Tree {
                    name: text(t, "name"),
                    title: text(t, "title"),
                    when: text(t, "when"),
                    boxes: t.get("boxes").and_then(Value::as_u64).unwrap_or(0),
                    broken: t.get("broken").and_then(Value::as_str).map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn json_bin() -> PathBuf {
    if let Some(bin) = std::env::var_os(ENV_JSON_BIN).filter(|v| !v.is_empty()) {
        return PathBuf::from(bin);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(JSON_BIN)))
        .unwrap_or_else(|| PathBuf::from(JSON_BIN))
}

pub fn edit_argv(bin: &std::path::Path, name: &str, addr: &str) -> Option<Vec<String>> {
    good_name(name).then(|| {
        vec![
            bin.display().to_string(),
            name.to_string(),
            "--brain".into(),
            addr.to_string(),
        ]
    })
}

pub fn tree_line(tree: &Tree) -> String {
    match &tree.broken {
        Some(problem) => format!("{}.json: {problem}", tree.name),
        None => {
            let when = if tree.when.is_empty() {
                "no when line".to_string()
            } else {
                tree.when.clone()
            };
            format!(
                "{}.json · {} {} · {when}",
                tree.name,
                tree.boxes,
                if tree.boxes == 1 { "box" } else { "boxes" }
            )
        }
    }
}

pub fn watch_label(minutes: u64) -> String {
    if minutes == 0 {
        "Off".into()
    } else {
        format!("{minutes} min")
    }
}

pub fn parse_deep(value: &Value) -> (Vec<DeepSeat>, Vec<DeepAsk>, Vec<DeepAsk>) {
    let view = value.get("view").unwrap_or(value);
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let seats = view
        .get("brains")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|row| {
                    let seat = row.get("seat").unwrap_or(row);
                    DeepSeat {
                        id: text(seat, "id"),
                        space_name: text(seat, "space_name"),
                        name: text(seat, "name"),
                        terminal: seat.get("terminal").and_then(Value::as_str).map(str::to_string),
                        state: text(row, "state"),
                        on: seat.get("on").and_then(Value::as_bool).unwrap_or(true),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let parse_asks = |key: &str| {
        view.get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|a| DeepAsk {
                        id: text(a, "id"),
                        space_name: text(a, "space_name"),
                        state: text(a, "state"),
                        question: text(a, "question"),
                        pick: a.get("pick").and_then(Value::as_str).map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    (seats, parse_asks("open"), parse_asks("recent"))
}

pub fn build(addr: String) -> RuleTreesView {
    RuleTreesView {
        addr,
        trees: None,
        error: None,
        watch: None,
        watch_error: None,
        deep_auto_start: None,
        deep_error: None,
        deep_seats: None,
        deep_open: Vec::new(),
        deep_recent: Vec::new(),
        deep_status_error: None,
        data_dir: None,
        name: "Nixa".into(),
    }
}

pub fn new(addr: String, cx: &mut App) -> Entity<RuleTreesView> {
    let view = cx.new(|_| build(addr));
    view.update(cx, reload);
    view
}

pub fn reload(view: &mut RuleTreesView, cx: &mut Context<RuleTreesView>) {
    load_trees(view, cx);
    load_settings(view, cx);
    load_deep(view, cx);
}

pub fn load_deep(view: &mut RuleTreesView, cx: &mut Context<RuleTreesView>) {
    let addr = view.addr.clone();
    call(
        &addr,
        "GET",
        "/deep".into(),
        None,
        cx,
        |view, result, _| match result {
            Ok(value) => {
                let (seats, open, recent) = parse_deep(&value);
                view.deep_seats = Some(seats);
                view.deep_open = open;
                view.deep_recent = recent;
                view.deep_status_error = None;
            }
            Err(error) => {
                view.deep_status_error = Some(format!("Deep brain status could not be read: {error}"));
            }
        },
    );
}

pub fn load_trees(view: &mut RuleTreesView, cx: &mut Context<RuleTreesView>) {
    let addr = view.addr.clone();
    call(
        &addr,
        "GET",
        "/trees".into(),
        None,
        cx,
        |view, result, _| match result {
            Ok(value) => {
                view.trees = Some(parse_trees(&value));
                view.error = None;
            }
            Err(error) => view.error = Some(format!("The rule files could not be read: {error}")),
        },
    );
}

pub fn load_settings(view: &mut RuleTreesView, cx: &mut Context<RuleTreesView>) {
    let addr = view.addr.clone();
    call(
        &addr,
        "GET",
        "/settings".into(),
        None,
        cx,
        |view, result, _| match result {
            Ok(value) => {
                view.watch = value.get("watchMinutes").and_then(Value::as_u64);
                view.deep_auto_start = deep_auto_start_of(&value);
                view.deep_error = None;
                view.data_dir = value
                    .get("dataDir")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(name) = value
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                {
                    view.name = name.to_string();
                }
                view.watch_error = None;
            }
            Err(error) => view.watch_error = Some(format!("Not connected: {error}")),
        },
    );
}

pub fn set_watch(view: &mut RuleTreesView, minutes: u64, cx: &mut Context<RuleTreesView>) {
    if view.watch == Some(minutes) {
        return;
    }
    let addr = view.addr.clone();
    call(
        &addr,
        "POST",
        "/settings".into(),
        Some(json!({"watchMinutes": minutes})),
        cx,
        move |view, result, _| match result {
            Ok(_) => {
                view.watch = Some(minutes);
                view.watch_error = None;
            }
            Err(error) => view.watch_error = Some(format!("Not changed: {error}")),
        },
    );
}

pub fn deep_auto_start_of(value: &Value) -> Option<bool> {
    value
        .get("settings")
        .and_then(|s| s.get("deepAutoStart"))
        .and_then(Value::as_bool)
}

pub fn deep_on(view: &RuleTreesView) -> bool {
    view.deep_auto_start.unwrap_or(true)
}

pub fn set_deep_auto_start(view: &mut RuleTreesView, on: bool, cx: &mut Context<RuleTreesView>) {
    let addr = view.addr.clone();
    call(
        &addr,
        "POST",
        "/settings".into(),
        Some(json!({"deepAutoStart": on})),
        cx,
        move |view, result, _| match result {
            Ok(value) => {
                view.deep_auto_start = deep_auto_start_of(&value).or(Some(on));
                view.deep_error = None;
            }
            Err(error) => view.deep_error = Some(format!("Not changed: {error}")),
        },
    );
}

pub fn edit_json(view: &mut RuleTreesView, name: &str, cx: &mut Context<RuleTreesView>) {
    if let Some(argv) = edit_argv(&json_bin(), name, &view.addr) {
        cx.emit(RuleTreesEvent::EditJson {
            name: name.to_string(),
            argv,
        });
    }
}

pub fn on_event(view: &mut RuleTreesView, msg: &Value, cx: &mut Context<RuleTreesView>) -> bool {
    match msg.get("type").and_then(Value::as_str) {
        Some("trees_changed") => {
            load_trees(view, cx);
            true
        }
        Some("deep") => {
            load_deep(view, cx);
            true
        }
        Some("hello") => {
            reload(view, cx);
            true
        }
        _ => false,
    }
}

pub fn feed(view: &Entity<RuleTreesView>, msg: &Value, cx: &mut App) -> bool {
    view.update(cx, |view, cx| on_event(view, msg, cx))
}

fn small(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Div {
    div()
        .w_full()
        .text_size(px(11.5))
        .text_color(color)
        .child(text.into())
}

fn heading(t: &Theme, text: &'static str) -> gpui::Div {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.bright)
        .child(text)
}

fn tree_row(t: &Theme, tree: &Tree, cx: &mut Context<RuleTreesView>) -> AnyElement {
    let name = tree.name.clone();
    let title = if tree.broken.is_some() {
        format!("{} (broken)", tree.title)
    } else {
        tree.title.clone()
    };
    div()
        .id(SharedString::from(format!("rule-file-{}", tree.name)))
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(if tree.broken.is_some() {
                            t.red
                        } else {
                            t.foreground
                        })
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(t.dim)
                        .child(tree_line(tree)),
                ),
        )
        .child(
            button(
                t,
                SharedString::from(format!("rule-file-edit-{}", tree.name)),
                "Edit JSON",
                ButtonKind::Secondary,
            )
            .on_click(cx.listener(move |view, _, _, cx| edit_json(view, &name, cx))),
        )
        .into_any_element()
}

fn watch_row(view: &RuleTreesView, t: &Theme, cx: &mut Context<RuleTreesView>) -> AnyElement {
    let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(
        div()
            .w(px(96.))
            .text_size(px(11.5))
            .text_color(t.dim)
            .child("Check every"),
    );
    for minutes in WATCH_CHOICES {
        let on = view.watch == Some(minutes);
        let hover = t.hover;
        row = row.child(
            div()
                .id(SharedString::from(format!("watch-{minutes}")))
                .h(px(24.))
                .px(px(10.))
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(if on { t.green } else { t.border })
                .when(on, |el| el.bg(t.hover).font_weight(FontWeight::SEMIBOLD))
                .text_size(px(11.5))
                .text_color(if on { t.green } else { t.foreground })
                .when(!on, |el| {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |view, _, _, cx| set_watch(view, minutes, cx)))
                })
                .child(watch_label(minutes)),
        );
    }
    row.into_any_element()
}

fn deep_status(view: &RuleTreesView, t: &Theme, cx: &mut Context<RuleTreesView>) -> AnyElement {
    let mut box_el = div()
        .id("deep-brain-status")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(5.))
        .p(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border);
    box_el = box_el.child(
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.foreground)
                    .child("Seats and requests"),
            )
            .child(
                button(t, "deep-refresh", "Refresh", ButtonKind::Secondary)
                    .on_click(cx.listener(|view, _, _, cx| load_deep(view, cx))),
            ),
    );
    if let Some(error) = &view.deep_status_error {
        return box_el.child(small(error.clone(), t.red)).into_any_element();
    }
    match &view.deep_seats {
        None => {
            box_el = box_el.child(small("Loading deep brain seats…", t.dim));
        }
        Some(seats) if seats.is_empty() => {
            box_el = box_el.child(small(
                "No deep brain seats yet. A seat starts automatically when a space needs one.",
                t.dim,
            ));
        }
        Some(seats) => {
            for seat in seats {
                let term = seat
                    .terminal
                    .as_deref()
                    .map(|t| format!(" · {t}"))
                    .unwrap_or_default();
                let line = format!(
                    "{} · {}{} — {}",
                    seat.space_name, seat.name, term, seat.state
                );
                let color = if !seat.on {
                    t.dim
                } else if seat.state.starts_with("up") {
                    t.green
                } else if seat.state.contains("down") {
                    t.red
                } else {
                    t.foreground
                };
                box_el = box_el.child(small(line, color));
            }
        }
    }
    if !view.deep_open.is_empty() {
        box_el = box_el.child(
            div()
                .mt(px(4.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.foreground)
                .child(format!("Open requests ({})", view.deep_open.len())),
        );
        for ask in &view.deep_open {
            let q: String = ask.question.chars().take(90).collect();
            box_el = box_el.child(small(
                format!("{} [{} · {}]: {q}", ask.id, ask.space_name, ask.state),
                t.foreground,
            ));
        }
    } else if !view.deep_recent.is_empty() {
        box_el = box_el.child(
            div()
                .mt(px(4.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.dim)
                .child("Recent answers"),
        );
        for ask in view.deep_recent.iter().take(3) {
            let pick = ask.pick.as_deref().unwrap_or("");
            let short: String = pick.chars().take(80).collect();
            box_el = box_el.child(small(
                format!("{} [{} · {}]: {short}", ask.id, ask.space_name, ask.state),
                t.dim,
            ));
        }
    }
    box_el.into_any_element()
}

fn deep_row(view: &RuleTreesView, t: &Theme, cx: &mut Context<RuleTreesView>) -> AnyElement {
    let on = deep_on(view);
    let note = format!("When a request needs a deep brain and none runs, {} starts one by itself. When off, no deep brain is started for a request; start one yourself.", view.name);
    toggle_row(t, "deep-auto-start", "Start deep brains by themselves", Some(note.into()), on)
        .debug_selector(|| "deep-auto-start".into())
        .on_click(cx.listener(move |view, _, _, cx| set_deep_auto_start(view, !on, cx)))
        .into_any_element()
}

impl Render for RuleTreesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let name = self.name.clone();
        let mut files = div()
            .id("rule-files")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.));
        match (&self.error, &self.trees) {
            (Some(error), _) => files = files.child(small(error.clone(), t.red)),
            (None, None) => files = files.child(small("Loading the rule files…", t.dim)),
            (None, Some(trees)) if trees.is_empty() => {
                files = files.child(small(
                    format!("No rule files yet. Ask {name} to make one."),
                    t.dim,
                ))
            }
            (None, Some(trees)) => {
                for tree in trees {
                    files = files.child(tree_row(&t, tree, cx));
                }
            }
        }
        let folder = self
            .data_dir
            .clone()
            .map(|d| format!("{d}/trees"))
            .unwrap_or_else(|| "the trees folder in the data folder".into());
        div()
            .id("rule-trees-page")
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(heading(&t, "Rule trees"))
            .child(small(format!("Rule trees are rules as JSON files. Each box in a file is one rule in plain English. {name} gets every tree when its brain starts, and again when one changes. It follows the tree whose \"when\" line fits the job."), t.dim))
            .child(files)
            .child(heading(&t, "Watch the agents").mt(px(8.)))
            .child(small(format!("Every few minutes {name} is woken to follow the tree \"watch-agents\" for each open task and agent terminal. It tells you only what a box says to report. Each check costs one brain turn. It is skipped while {name} is off or work is on hold."), t.dim))
            .child(watch_row(self, &t, cx))
            .when_some(self.watch_error.clone(), |el, e| el.child(small(e, t.red)))
            .child(heading(&t, "Deep brain").mt(px(8.)))
            .child(deep_row(self, &t, cx))
            .when_some(self.deep_error.clone(), |el, e| el.child(small(e, t.red)))
            .child(deep_status(self, &t, cx))
            .child(small(format!("Files: {folder}/<name>.json. Edit JSON opens the tree in the JSON editor in a new terminal tab; it saves through the brain, which checks every edit, logs it and keeps history for undo. {name} and agents use the nixa_trees tool."), t.dim).mt(px(6.)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trees_parse_and_lines() {
        let value = json!({"trees": [
            {"name": "watch-agents", "title": "Watch", "when": "every few minutes", "boxes": 4},
            {"name": "bad-one", "title": "bad-one", "when": "", "boxes": 0, "broken": "Invalid JSON"},
            {"name": "../x", "title": "x"}
        ]});
        let trees = parse_trees(&value);
        assert_eq!(trees.len(), 2);
        assert_eq!(
            tree_line(&trees[0]),
            "watch-agents.json · 4 boxes · every few minutes"
        );
        assert_eq!(tree_line(&trees[1]), "bad-one.json: Invalid JSON");
        assert_eq!(watch_label(0), "Off");
        assert_eq!(watch_label(15), "15 min");
    }

    #[test]
    fn edit_json_argv_is_safe() {
        let bin = std::path::Path::new("/x/nixa-json");
        assert_eq!(
            edit_argv(bin, "watch-agents", "127.0.0.1:7421").unwrap(),
            vec!["/x/nixa-json", "watch-agents", "--brain", "127.0.0.1:7421"]
        );
        assert!(edit_argv(bin, "../etc", "a").is_none());
        assert!(edit_argv(bin, "Big", "a").is_none());
        assert!(edit_argv(bin, "", "a").is_none());
        assert!(good_name("a1-b"));
        assert!(!good_name(&"a".repeat(61)));
    }

    #[test]
    fn deep_auto_start_is_on_when_not_set() {
        let mut view = build("127.0.0.1:1".into());
        assert!(deep_on(&view));
        view.deep_auto_start = deep_auto_start_of(&json!({"settings": {"deepAutoStart": null}}));
        assert!(deep_on(&view));
        view.deep_auto_start = deep_auto_start_of(&json!({"settings": {"deepAutoStart": false}}));
        assert!(!deep_on(&view));
        view.deep_auto_start = deep_auto_start_of(&json!({"settings": {"deepAutoStart": true}}));
        assert!(deep_on(&view));
    }

    #[test]
    fn deep_status_parses_seats_and_requests() {
        let payload = json!({
            "view": {
                "brains": [
                    {"seat": {"id": "s1", "space_name": "nixa", "name": "Claude", "terminal": "nixa-claude-1", "on": true}, "state": "up, free"}
                ],
                "open": [
                    {"id": "d1", "space_name": "nixa", "state": "sent", "question": "Pick agent for task #4"}
                ],
                "recent": [
                    {"id": "d0", "space_name": "nixa", "state": "answered", "question": "Old", "pick": "use codex"}
                ]
            }
        });
        let (seats, open, recent) = parse_deep(&payload);
        assert_eq!(seats.len(), 1);
        assert_eq!(seats[0].space_name, "nixa");
        assert_eq!(seats[0].terminal.as_deref(), Some("nixa-claude-1"));
        assert_eq!(seats[0].state, "up, free");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "d1");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].pick.as_deref(), Some("use codex"));
    }
}
