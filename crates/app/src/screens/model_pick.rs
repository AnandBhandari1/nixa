use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Entity, EventEmitter, Focusable, ScrollHandle, SharedString, Window, div, px};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::theme;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const MAX_ROWS: usize = 60;
pub const MAX_LIST_HEIGHT: f32 = 300.;
pub const ACTION: &str = "agent_models";
pub const REFRESH_ALL: &str = "refresh_lists";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listed {
    pub models: Vec<String>,
    pub efforts: BTreeMap<String, Vec<String>>,
    pub at: i64,
    pub error: String,
}

pub type Load = Arc<dyn Fn(&str, bool) -> Result<Listed, String> + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Default,
    Model { name: String, hint: String },
    Typed(String),
}

pub enum ModelPickEvent {
    Picked(String),
}

pub struct ModelPick {
    pub id: String,
    pub agent: String,
    pub value: String,
    pub extra: Vec<String>,
    pub default_label: String,
    pub listed: Listed,
    pub loading: bool,
    pub error: Option<String>,
    pub open: bool,
    pub filter: Entity<TextInput>,
    scroll: ScrollHandle,
    load: Load,
    loads: u64,
}

impl EventEmitter<ModelPickEvent> for ModelPick {}

pub fn cache_key(agent: &str) -> &str {
    match agent.trim() {
        "antigravity" => "agy",
        "cursor-agent" => "cursor",
        other => other,
    }
}

pub fn listed_of(value: &Value) -> Listed {
    let data = value.get("data").unwrap_or(value);
    let models = data.get("models").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let efforts = data.get("efforts").and_then(|e| serde_json::from_value(e.clone()).ok()).unwrap_or_default();
    Listed { models, efforts, at: data.get("at").and_then(Value::as_i64).unwrap_or(0), error: data.get("error").and_then(Value::as_str).unwrap_or_default().to_string() }
}

pub fn has_list(agent: &str) -> bool {
    nixa_store::models::lists::LIST_AGENTS.contains(&cache_key(agent))
}

pub fn brain_list(agent: &str, refresh: bool) -> Result<Listed, String> {
    if !has_list(agent) {
        return Ok(Listed::default());
    }
    let body = json!({"agent": cache_key(agent), "refresh": refresh});
    crate::screens::models::api::brain()(ACTION, Some(body)).map(|v| listed_of(&v))
}

pub fn brain_load() -> Load {
    Arc::new(brain_list)
}

pub fn load_of(list: fn(&str, bool) -> Result<Listed, String>) -> Load {
    Arc::new(list)
}

pub fn efforts_of(listed: &Listed, model: &str) -> Vec<String> {
    listed.efforts.get(model).or_else(|| listed.efforts.get("*")).cloned().unwrap_or_default().into_iter().filter(|e| e != "default").collect()
}

pub fn rows(listed: &Listed, extra: &[String], filter: &str) -> Vec<Row> {
    let wanted = filter.trim().to_lowercase();
    let mut names: Vec<String> = Vec::new();
    for name in listed.models.iter().chain(extra.iter()) {
        if !name.trim().is_empty() && !names.contains(name) {
            names.push(name.clone());
        }
    }
    let mut out = Vec::new();
    if wanted.is_empty() {
        out.push(Row::Default);
    }
    for name in names.iter().filter(|n| wanted.is_empty() || n.to_lowercase().contains(&wanted)).take(MAX_ROWS) {
        let efforts = efforts_of(listed, name);
        let hint = if efforts.is_empty() { String::new() } else { format!("effort: {}", efforts.join(", ")) };
        out.push(Row::Model { name: name.clone(), hint });
    }
    let typed = filter.trim();
    if !typed.is_empty() && !names.iter().any(|n| n == typed) {
        out.push(Row::Typed(typed.to_string()));
    }
    out
}

pub fn shown(value: &str, default_label: &str) -> String {
    if value.trim().is_empty() { default_label.to_string() } else { value.to_string() }
}

pub fn age(at: i64, now: i64) -> String {
    if at <= 0 {
        return "not read yet".into();
    }
    let minutes = (now - at).max(0) / 60_000;
    match minutes {
        0 => "read just now".into(),
        m if m < 60 => format!("read {m} min ago"),
        m if m < 48 * 60 => format!("read {} h ago", m / 60),
        m => format!("read {} days ago", m / (24 * 60)),
    }
}

pub fn new(id: &str, agent: &str, value: &str, load: Load, cx: &mut App) -> Entity<ModelPick> {
    let id = id.to_string();
    let agent = agent.to_string();
    let value = value.to_string();
    cx.new(|cx| {
        let filter = cx.new(|cx| TextInput::new(cx, "Search or type a model name"));
        cx.subscribe(&filter, |v: &mut ModelPick, input, e: &InputEvent, cx| match e {
            InputEvent::Changed(_) => {
                v.scroll.set_offset(gpui::point(px(0.), px(0.)));
                cx.notify();
            }
            InputEvent::Submit(_) => {
                let typed = input.read(cx).value().trim().to_string();
                if let Some(row) = rows(&v.listed, &v.extra, &typed).into_iter().find(|r| !matches!(r, Row::Default)) {
                    v.choose(row, cx);
                }
            }
            _ => {}
        })
        .detach();
        let mut view = ModelPick { id, agent, value, extra: Vec::new(), default_label: "Default model".into(), listed: Listed::default(), loading: false, error: None, open: false, filter, scroll: ScrollHandle::new(), load, loads: 0 };
        view.reload(false, cx);
        view
    })
}

impl ModelPick {
    pub fn set_agent(&mut self, agent: &str, value: &str, cx: &mut Context<Self>) {
        let changed = self.agent != agent;
        self.agent = agent.to_string();
        self.value = value.to_string();
        if changed {
            self.listed = Listed::default();
            self.extra.clear();
            self.reload(false, cx);
        }
        cx.notify();
    }

    pub fn set_extra(&mut self, extra: Vec<String>, cx: &mut Context<Self>) {
        self.extra = extra;
        cx.notify();
    }

    pub fn efforts(&self) -> Vec<String> {
        efforts_of(&self.listed, &self.value)
    }

    pub fn reload(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.agent.trim().is_empty() {
            return;
        }
        self.loads += 1;
        let load_id = self.loads;
        self.loading = true;
        self.error = None;
        let load = self.load.clone();
        let agent = self.agent.clone();
        let task = cx.background_executor().spawn(async move { load(&agent, refresh) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |v, cx| {
                if v.loads != load_id {
                    return;
                }
                v.loading = false;
                match result {
                    Ok(listed) => {
                        v.error = (!listed.error.is_empty()).then(|| listed.error.clone());
                        v.listed = listed;
                    }
                    Err(why) => v.error = Some(why),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn choose(&mut self, row: Row, cx: &mut Context<Self>) {
        let value = match row {
            Row::Default => String::new(),
            Row::Model { name, .. } => name,
            Row::Typed(name) => name,
        };
        self.value = value.clone();
        self.open = false;
        self.filter.update(cx, |f, cx| f.set_value("", cx));
        cx.emit(ModelPickEvent::Picked(value));
        cx.notify();
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = !self.open;
        if self.open {
            if self.listed.models.is_empty() && !self.loading {
                self.reload(false, cx);
            }
            let focus = self.filter.read(cx).focus_handle(cx);
            window.focus(&focus);
        }
        cx.notify();
    }
}

impl Render for ModelPick {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let id = self.id.clone();
        let label = shown(&self.value, &self.default_label);
        let head = div()
            .id(SharedString::from(format!("{id}-button")))
            .debug_selector({
                let id = id.clone();
                move || format!("{id}-button")
            })
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(10.))
            .py(px(7.))
            .rounded(px(6.))
            .border_1()
            .border_color(if self.open { t.ring } else { t.border })
            .bg(t.input)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(div().truncate().text_size(px(13.)).text_color(if self.value.is_empty() { t.dim } else { t.foreground }).child(label))
            .child(div().text_size(px(11.)).text_color(t.dim).child(if self.open { "▴" } else { "▾" }))
            .on_click(cx.listener(|v, _: &ClickEvent, window, cx| v.toggle(window, cx)));
        let mut col = div().w_full().flex().flex_col().gap(px(4.)).child(head);
        if !self.open {
            return col;
        }
        let filter = self.filter.read(cx).value().to_string();
        let mut list = div().flex().flex_col().gap(px(1.)).pr(px(8.));
        for (i, row) in rows(&self.listed, &self.extra, &filter).into_iter().enumerate() {
            let (main, hint, on) = match &row {
                Row::Default => (self.default_label.clone(), "Let the tool choose".to_string(), self.value.is_empty()),
                Row::Model { name, hint } => (name.clone(), hint.clone(), *name == self.value),
                Row::Typed(name) => (format!("Use \"{name}\""), "Not in the list; saved as typed".to_string(), false),
            };
            let tag = format!("{id}-row-{i}");
            let pick = row.clone();
            list = list.child(
                div()
                    .id(SharedString::from(tag.clone()))
                    .debug_selector(move || tag.clone())
                    .flex_none()
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .items_center()
                    .cursor_pointer()
                    .when(on, |el| el.bg(t.selected))
                    .hover(|s| s.bg(t.hover))
                    .child(div().w(px(14.)).flex_none().text_color(t.green).child(if on { "✓" } else { "" }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.))
                            .child(div().truncate().text_size(px(12.5)).child(main))
                            .when(!hint.is_empty(), |el| el.child(div().truncate().text_size(px(10.5)).text_color(t.dim).child(hint))),
                    )
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.choose(pick.clone(), cx))),
            );
        }
        let state = if self.loading {
            "Reading the model list…".to_string()
        } else if let Some(error) = &self.error {
            format!("Could not read the list: {error}")
        } else if !has_list(&self.agent) {
            "No shared list for this tool. Pick one below or type a name.".to_string()
        } else {
            age(self.listed.at, chrono_now())
        };
        let refresh_id = format!("{id}-refresh");
        let foot = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .pt(px(4.))
            .child(div().text_size(px(10.5)).text_color(if self.error.is_some() { t.red } else { t.dim }).child(state))
            .child(
                div()
                    .id(SharedString::from(refresh_id.clone()))
                    .debug_selector(move || refresh_id.clone())
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child("Refresh list")
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.reload(true, cx))),
            );
        let panel = div().p(px(8.)).flex().flex_col().gap(px(6.)).rounded(px(8.)).border_1().border_color(t.border).bg(t.surface).child(self.filter.clone())
            .child(
                nixa_ui::scroll::scroll_area(&t, SharedString::from(format!("{id}-list")), &self.scroll, list)
                    .max_h(px(MAX_LIST_HEIGHT))
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation()),
            )
            .child(foot);
        col = col.child(panel);
        col
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Listed {
        Listed {
            models: vec!["claude-opus-5-5".into(), "claude-haiku-5-5".into()],
            efforts: BTreeMap::from([("*".to_string(), vec!["default".to_string(), "low".to_string(), "high".to_string()])]),
            at: 1,
            error: String::new(),
        }
    }

    #[test]
    fn rows_start_with_default_then_models_then_a_typed_name() {
        let all = rows(&listed(), &["llama3".into(), "claude-haiku-5-5".into()], "");
        assert_eq!(all[0], Row::Default);
        assert_eq!(all.len(), 4, "the extra list adds llama3 once and skips the repeat");
        assert_eq!(all[1], Row::Model { name: "claude-opus-5-5".into(), hint: "effort: low, high".into() });
        let found = rows(&listed(), &[], "haiku");
        assert_eq!(found.len(), 2);
        assert!(matches!(&found[0], Row::Model { name, .. } if name == "claude-haiku-5-5"));
        assert_eq!(found[1], Row::Typed("haiku".into()));
        assert_eq!(rows(&listed(), &[], "claude-opus-5-5").len(), 1, "an exact name is not offered again as typed");
    }

    #[test]
    fn keys_values_and_ages_read_plainly() {
        assert_eq!(cache_key("antigravity"), "agy");
        assert_eq!(cache_key("cursor-agent"), "cursor");
        assert_eq!(cache_key("codex"), "codex");
        assert_eq!(shown("", "Default model"), "Default model");
        assert_eq!(shown("gpt-6", "Default model"), "gpt-6");
        assert_eq!(age(0, 5), "not read yet");
        assert_eq!(age(1_000, 1_000 + 5 * 60_000), "read 5 min ago");
        assert_eq!(age(1_000, 1_000 + 3 * 24 * 60 * 60_000), "read 3 days ago");
        let parsed = listed_of(&json!({"ok": true, "data": {"models": ["a", "b"], "efforts": {"a": ["low"]}, "at": 7}}));
        assert_eq!(parsed.models, vec!["a", "b"]);
        assert_eq!(efforts_of(&parsed, "a"), vec!["low"]);
        assert_eq!(parsed.at, 7);
    }
}
