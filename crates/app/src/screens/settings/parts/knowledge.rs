use super::outside::{self, Call, card, field, flag, note, row, text, title};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, SharedString, Window, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::text_input::TextInput;
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};

pub const PART: &str = "Knowledge";
pub const STATS_PATH: &str = "/knowledge";
pub const INDEX_PATH: &str = "/knowledge/index";
pub const FOLDERS_PATH: &str = "/knowledge/folders";
pub const SKILLS_PATH: &str = "/knowledge/skills";
pub const ABOUT: &str = "Nixa searches its memory, the chat, the docs folders of your spaces and the notes folders you add here. Everything stays on this Mac.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub chunks: u64,
    pub indexing: bool,
    pub last_at: String,
    pub last_error: String,
    pub files: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Folders,
    Skills,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

pub struct KnowledgeView {
    pub call: Call,
    pub stats: Option<Stats>,
    pub folders: Vec<String>,
    pub skills: Vec<Skill>,
    pub allowed: Vec<String>,
    pub folder: Entity<TextInput>,
    pub indexing: bool,
    pub error: Option<String>,
    pub note: Option<String>,
    pub saving: Vec<List>,
    pub again: Vec<List>,
}

fn strings(v: &Value, key: &str) -> Option<Vec<String>> {
    v.get(key)?.as_array().map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

pub fn stats_of(reply: &Value) -> Option<Stats> {
    let k = reply.get("knowledge").filter(|k| k.is_object())?;
    let h = k.get("health").cloned().unwrap_or(Value::Null);
    Some(Stats {
        chunks: k.get("chunks").and_then(Value::as_u64).unwrap_or(0),
        indexing: flag(k, "indexing"),
        last_at: text(&h, "last_index_at"),
        last_error: text(&h, "last_index_error"),
        files: h.get("last_index_files").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub fn folders_of(reply: &Value) -> Option<Vec<String>> {
    strings(reply, "folders").or_else(|| reply.get("knowledge").and_then(|k| strings(k, "folders")))
}

pub fn skills_of(reply: &Value) -> Option<(Vec<Skill>, Vec<String>)> {
    let s = reply.get("skills").filter(|s| s.is_object())?;
    let installed = s.get("installed").and_then(Value::as_array).into_iter().flatten().map(|i| Skill { name: text(i, "name"), description: text(i, "description") }).filter(|i| !i.name.is_empty()).collect();
    Some((installed, strings(s, "allowed").unwrap_or_default()))
}

pub fn with_folder(list: &[String], folder: &str) -> Result<Vec<String>, String> {
    let folder = folder.trim().trim_end_matches('/');
    if folder.is_empty() {
        return Err("Type a full folder path first.".into());
    }
    if !folder.starts_with('/') && !folder.starts_with("~/") {
        return Err("Use a full folder path, for example ~/Notes.".into());
    }
    let mut out = list.to_vec();
    if !out.iter().any(|f| f == folder) {
        out.push(folder.to_string());
    }
    Ok(out)
}

pub fn without(list: &[String], item: &str) -> Vec<String> {
    list.iter().filter(|f| *f != item).cloned().collect()
}

pub fn toggled(list: &[String], name: &str, on: bool) -> Vec<String> {
    let mut out = without(list, name);
    if on {
        out.push(name.to_string());
    }
    out
}

pub fn stats_line(s: &Stats) -> String {
    let state = if s.indexing { " Indexing now." } else { "" };
    let last = if s.last_at.is_empty() { " Not indexed yet.".to_string() } else { format!(" Last index: {}, {} files.", s.last_at, s.files) };
    let error = if s.last_error.is_empty() { String::new() } else { format!(" Last problem: {}.", s.last_error) };
    format!("{} parts in the index.{last}{state}{error}", s.chunks)
}

pub fn build(call: Call, cx: &mut Context<KnowledgeView>) -> KnowledgeView {
    KnowledgeView {
        call,
        stats: None,
        folders: Vec::new(),
        skills: Vec::new(),
        allowed: Vec::new(),
        folder: cx.new(|cx| TextInput::new(cx, "A notes folder, for example ~/Notes")),
        indexing: false,
        error: None,
        note: None,
        saving: Vec::new(),
        again: Vec::new(),
    }
}

pub fn take(v: &mut KnowledgeView, reply: &Value) -> bool {
    let mut took = false;
    if let Some(stats) = stats_of(reply) {
        v.stats = Some(stats);
        took = true;
    }
    if let Some(folders) = folders_of(reply) {
        v.folders = folders;
        took = true;
    }
    if let Some((skills, allowed)) = skills_of(reply) {
        v.skills = skills;
        v.allowed = allowed;
        took = true;
    }
    took
}

fn send(v: &mut KnowledgeView, method: &'static str, path: &'static str, body: Option<Value>, done: String, cx: &mut Context<KnowledgeView>) {
    if !done.is_empty() {
        v.note = None;
    }
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { call(method, path, body) });
    cx.spawn(async move |this, cx| {
        let reply = task.await;
        this.update(cx, |v, cx| {
            if path == INDEX_PATH {
                v.indexing = false;
            }
            match reply {
                Ok(reply) => match outside::error_of(&reply) {
                    Some(e) => v.error = Some(e),
                    None => {
                        v.error = None;
                        take(v, &reply);
                        if !done.is_empty() {
                            v.note = Some(done);
                        }
                    }
                },
                Err(e) => v.error = Some(e),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn reload(v: &mut KnowledgeView, cx: &mut Context<KnowledgeView>) {
    send(v, "GET", STATS_PATH, None, String::new(), cx);
    send(v, "GET", SKILLS_PATH, None, String::new(), cx);
}

pub fn body_of(v: &KnowledgeView, list: List) -> (&'static str, Value) {
    match list {
        List::Folders => (FOLDERS_PATH, json!({"folders": v.folders})),
        List::Skills => (SKILLS_PATH, json!({"allowed": v.allowed})),
    }
}

pub fn save(v: &mut KnowledgeView, list: List, done: &'static str, cx: &mut Context<KnowledgeView>) {
    v.note = None;
    if v.saving.contains(&list) {
        if !v.again.contains(&list) {
            v.again.push(list);
        }
        cx.notify();
        return;
    }
    v.saving.push(list);
    let (path, body) = body_of(v, list);
    let call = v.call.clone();
    let task = cx.background_executor().spawn(async move { call("POST", path, Some(body)) });
    cx.spawn(async move |this, cx| {
        let reply = task.await;
        this.update(cx, |v, cx| {
            v.saving.retain(|l| *l != list);
            let again = v.again.contains(&list);
            v.again.retain(|l| *l != list);
            match reply.and_then(|r| outside::error_of(&r).map_or(Ok(r), Err)) {
                Ok(_) if again => save(v, list, done, cx),
                Ok(reply) => {
                    v.error = None;
                    take(v, &reply);
                    v.note = Some(done.into());
                }
                Err(e) => {
                    v.error = Some(e);
                    reload(v, cx);
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn add_folder(v: &mut KnowledgeView, cx: &mut Context<KnowledgeView>) {
    match with_folder(&v.folders, v.folder.read(cx).value()) {
        Ok(list) => {
            v.folder.update(cx, |i, cx| i.set_value("", cx));
            v.folders = list;
            save(v, List::Folders, "Folder added. It is read at the next index.", cx);
        }
        Err(e) => {
            v.error = Some(e);
            cx.notify();
        }
    }
}

pub fn remove_folder(v: &mut KnowledgeView, folder: String, cx: &mut Context<KnowledgeView>) {
    v.folders = without(&v.folders, &folder);
    save(v, List::Folders, "Folder removed.", cx);
}

pub fn index_now(v: &mut KnowledgeView, cx: &mut Context<KnowledgeView>) {
    if v.indexing {
        return;
    }
    v.indexing = true;
    send(v, "POST", INDEX_PATH, Some(json!({})), "Index done.".into(), cx);
}

pub fn set_skill(v: &mut KnowledgeView, name: String, on: bool, cx: &mut Context<KnowledgeView>) {
    v.allowed = toggled(&v.allowed, &name, on);
    save(v, List::Skills, "Saved.", cx);
}

pub fn new(call: Call, cx: &mut App) -> Entity<KnowledgeView> {
    cx.new(|cx| {
        let mut v = build(call, cx);
        reload(&mut v, cx);
        v
    })
}

pub struct Shared(pub Entity<KnowledgeView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<KnowledgeView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<KnowledgeView> {
    if let Some(view) = shared(cx) {
        view.update(cx, reload);
        return view;
    }
    let view = new(outside::brain(), cx);
    cx.set_global(Shared(view.clone()));
    view
}

fn folder_row(t: &Theme, n: usize, folder: &str, cx: &mut Context<KnowledgeView>) -> AnyElement {
    let gone = folder.to_string();
    let tag = format!("knowledge-remove-{n}");
    row()
        .w_full()
        .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(t.foreground).child(folder.to_string()))
        .child(button(t, SharedString::from(tag.clone()), "Remove", ButtonKind::Ghost).debug_selector(move || tag.clone()).on_click(cx.listener(move |v, _, _, cx| remove_folder(v, gone.clone(), cx))))
        .into_any_element()
}

pub fn body(v: &KnowledgeView, t: &Theme, cx: &mut Context<KnowledgeView>) -> AnyElement {
    let mut folders = card(t).child(title(t, "Notes folders")).child(note(t.dim, 11.5, "Add a folder of your own notes. Hidden folders, secrets and private settings are never read."));
    if v.folders.is_empty() {
        folders = folders.child(div().debug_selector(|| "knowledge-no-folders".into()).child(note(t.muted, 12., "No notes folders yet.")));
    }
    for (n, folder) in v.folders.iter().enumerate() {
        folders = folders.child(folder_row(t, n, folder, cx));
    }
    folders = folders
        .child(field(t, "Folder", &v.folder, "knowledge-folder", ""))
        .child(row().child(button(t, "knowledge-add", "Add folder", ButtonKind::Secondary).debug_selector(|| "knowledge-add".into()).on_click(cx.listener(|v, _, _, cx| add_folder(v, cx)))));
    let index = card(t)
        .child(title(t, "Index"))
        .child(note(t.dim, 11.5, v.stats.as_ref().map(stats_line).unwrap_or_else(|| "Asking Nixa…".into())))
        .child(
            row().child(
                button(t, "knowledge-index", if v.indexing || v.stats.as_ref().is_some_and(|s| s.indexing) { "Indexing…" } else { "Index now" }, ButtonKind::Secondary)
                    .debug_selector(|| "knowledge-index".into())
                    .on_click(cx.listener(|v, _, _, cx| index_now(v, cx))),
            ),
        );
    let mut skills = card(t).child(title(t, "Skills Nixa may read")).child(note(t.dim, 11.5, "Skills are the agent skill folders on this Mac. Nixa reads only the ones you turn on."));
    if v.skills.is_empty() {
        skills = skills.child(note(t.muted, 12., "No skills found."));
    }
    for s in &v.skills {
        let on = v.allowed.contains(&s.name);
        let name = s.name.clone();
        let tag = format!("knowledge-skill-{name}");
        let about = (!s.description.is_empty()).then(|| SharedString::from(s.description.clone()));
        skills = skills.child(toggle_row(t, SharedString::from(tag.clone()), s.name.clone(), about, on).debug_selector(move || tag.clone()).on_click(cx.listener(move |v, _, _, cx| set_skill(v, name.clone(), !on, cx))));
    }
    div()
        .id("knowledge-part")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(note(t.dim, 11.5, ABOUT))
        .child(folders)
        .child(index)
        .child(skills)
        .when_some(v.note.clone(), |el, n| el.child(note(t.green, 12., n)))
        .when_some(v.error.clone(), |el, e| el.child(note(t.yellow, 12., e)))
        .into_any_element()
}

impl Render for KnowledgeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("settings-knowledge-scroll")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .child(body(self, &t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};
    use std::sync::{Arc, Mutex};

    #[test]
    fn folder_lists_and_stats() {
        assert_eq!(with_folder(&[], " ~/Notes/ ").unwrap(), vec!["~/Notes"]);
        assert_eq!(with_folder(&["/a".into()], "/a").unwrap(), vec!["/a"]);
        assert!(with_folder(&[], "notes").is_err());
        assert!(with_folder(&[], "  ").is_err());
        assert_eq!(without(&["/a".into(), "/b".into()], "/a"), vec!["/b"]);
        assert_eq!(toggled(&["x".into()], "y", true), vec!["x", "y"]);
        assert_eq!(toggled(&["x".into(), "y".into()], "x", false), vec!["y"]);
        let stats = stats_of(&json!({"ok": true, "knowledge": {"chunks": 12, "indexing": false, "folders": [], "health": {"last_index_at": "2026-01-02 10:00", "last_index_files": 3}}})).unwrap();
        assert_eq!(stats_line(&stats), "12 parts in the index. Last index: 2026-01-02 10:00, 3 files.");
        assert_eq!(stats_line(&Stats::default()), "0 parts in the index. Not indexed yet.");
        assert_eq!(folders_of(&json!({"knowledge": {"folders": ["/n"]}})), Some(vec!["/n".to_string()]));
        let (skills, allowed) = skills_of(&json!({"skills": {"installed": [{"name": "tdd", "description": "Test first"}], "allowed": ["tdd"]}})).unwrap();
        assert_eq!(skills, vec![Skill { name: "tdd".into(), description: "Test first".into() }]);
        assert_eq!(allowed, vec!["tdd"]);
    }

    #[gpui::test]
    fn quick_skill_switches_keep_every_change(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let allowed = Arc::new(Mutex::new(Vec::<String>::new()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (a, log) = (allowed.clone(), sent.clone());
        let call: Call = Arc::new(move |_, path, body| {
            if let Some(list) = body.as_ref().and_then(|b| b.get("allowed")).and_then(Value::as_array) {
                log.lock().unwrap().push(body.clone().unwrap_or_default());
                *a.lock().unwrap() = list.iter().filter_map(Value::as_str).map(str::to_string).collect();
            }
            let now = a.lock().unwrap().clone();
            Ok(match path {
                SKILLS_PATH => json!({"ok": true, "skills": {"installed": [{"name": "tdd"}, {"name": "docs"}], "allowed": now}}),
                _ => json!({"ok": true, "knowledge": {"chunks": 0, "indexing": false, "folders": [], "health": {}}}),
            })
        });
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let mut v = build(call, cx);
            reload(&mut v, cx);
            v
        });
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            view.update(cx, |v, cx| {
                set_skill(v, "tdd".into(), true, cx);
                set_skill(v, "docs".into(), true, cx);
            })
        });
        vcx.run_until_parked();
        assert_eq!(*allowed.lock().unwrap(), vec!["tdd", "docs"]);
        view.read_with(vcx, |v, _| {
            assert_eq!(v.allowed, vec!["tdd", "docs"]);
            assert!(v.saving.is_empty() && v.again.is_empty());
        });
        assert_eq!(*sent.lock().unwrap(), vec![json!({"allowed": ["tdd"]}), json!({"allowed": ["tdd", "docs"]})]);
        vcx.update(|_, cx| {
            view.update(cx, |v, cx| {
                set_skill(v, "tdd".into(), false, cx);
                set_skill(v, "docs".into(), false, cx);
            })
        });
        vcx.run_until_parked();
        assert!(allowed.lock().unwrap().is_empty(), "turning both off quickly leaves none on");
    }

    #[gpui::test]
    fn the_part_starts_empty_and_adds_removes_and_indexes(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let folders = Arc::new(Mutex::new(Vec::<String>::new()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (f, log) = (folders.clone(), sent.clone());
        let call: Call = Arc::new(move |method, path, body| {
            log.lock().unwrap().push((format!("{method} {path}"), body.clone()));
            if path == FOLDERS_PATH && let Some(list) = body.as_ref().and_then(|b| b.get("folders")).and_then(Value::as_array) {
                *f.lock().unwrap() = list.iter().filter_map(Value::as_str).map(str::to_string).collect();
            }
            let now = f.lock().unwrap().clone();
            Ok(match path {
                SKILLS_PATH => json!({"ok": true, "skills": {"installed": [{"name": "tdd", "description": "Test first"}], "allowed": []}}),
                FOLDERS_PATH => json!({"ok": true, "folders": now}),
                _ => json!({"ok": true, "knowledge": {"chunks": 0, "indexing": false, "folders": now, "health": {}}}),
            })
        });
        let (view, vcx) = cx.add_window_view(|_, cx| {
            let mut v = build(call, cx);
            reload(&mut v, cx);
            v
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("knowledge-no-folders").is_some(), "a fresh install has no notes folders");
        assert!(vcx.debug_bounds("knowledge-skill-tdd").is_some());
        vcx.update(|_, cx| view.update(cx, |v, cx| v.folder.update(cx, |i, cx| i.set_value("/Users/someone/Notes", cx))));
        let add = vcx.debug_bounds("knowledge-add").unwrap();
        vcx.simulate_click(add.center(), Modifiers::none());
        vcx.run_until_parked();
        view.read_with(vcx, |v, _| assert_eq!(v.folders, vec!["/Users/someone/Notes"]));
        vcx.update(|window, app| window.draw(app).clear());
        let remove = vcx.debug_bounds("knowledge-remove-0").unwrap();
        vcx.simulate_click(remove.center(), Modifiers::none());
        vcx.run_until_parked();
        view.read_with(vcx, |v, _| assert!(v.folders.is_empty()));
        let skill = vcx.debug_bounds("knowledge-skill-tdd").unwrap();
        vcx.simulate_click(skill.center(), Modifiers::none());
        let index = vcx.debug_bounds("knowledge-index").unwrap();
        vcx.simulate_click(index.center(), Modifiers::none());
        vcx.run_until_parked();
        let posts: Vec<(String, Value)> = sent.lock().unwrap().iter().filter_map(|(m, b)| Some((m.clone(), b.clone()?))).collect();
        assert_eq!(posts, vec![
            ("POST /knowledge/folders".to_string(), json!({"folders": ["/Users/someone/Notes"]})),
            ("POST /knowledge/folders".to_string(), json!({"folders": []})),
            ("POST /knowledge/skills".to_string(), json!({"allowed": ["tdd"]})),
            ("POST /knowledge/index".to_string(), json!({})),
        ]);
    }
}
