use super::memory::doc::{self, DocEditor, Document, call};
use super::memory::text_area::{AreaEvent, TextArea};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Global, SharedString, Subscription,
    Window, div, px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::chip::chip;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};

pub const MAX_TEXT: usize = 400;
pub const STATUSES: [&str; 3] = ["trial", "live", "off"];

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub id: u64,
    pub project: String,
    pub ask: String,
    pub answer: String,
    pub status: String,
    pub hits: u64,
    pub answered: u64,
    pub stamp: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub id: Option<u64>,
    pub status: String,
    pub expected: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormDraft {
    pub form: Option<Form>,
    pub project: String,
    pub ask: String,
    pub answer: String,
}

#[derive(Default)]
struct RuleDraft(FormDraft);

impl Global for RuleDraft {}

pub struct AnswerRulesView {
    pub addr: String,
    pub folder: Option<String>,
    pub rules_doc: Entity<DocEditor>,
    pub rules: Option<Vec<Rule>>,
    pub list_error: Option<String>,
    pub form: Option<Form>,
    pub project: Entity<TextInput>,
    pub ask: Entity<TextArea>,
    pub answer: Entity<TextArea>,
    pub form_notice: String,
    pub saving: bool,
    pub remove_ask: Option<u64>,
    pub notice: String,
    _watch: Vec<Subscription>,
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "trial" => "Trial (observe only)",
        "live" => "Live (answer agents)",
        "offer" => "Offer",
        _ => "Off",
    }
}

pub fn status_word(status: &str) -> &'static str {
    match status {
        "trial" => "Trial",
        "live" => "Live",
        "offer" => "Offer",
        _ => "Off",
    }
}

pub fn project_label(project: &str) -> String {
    if project == "*" {
        "All projects".into()
    } else {
        project.to_string()
    }
}

pub fn stamp_of(project: &str, ask: &str, answer: &str, status: &str) -> String {
    json!([project, ask, answer, status]).to_string()
}

pub fn parse_rules(value: &Value) -> Vec<Rule> {
    let stamps: Vec<&str> = value
        .get("stamps")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let text = |r: &Value, k: &str| {
        r.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    value
        .get("rules")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    let id = r.get("id").and_then(Value::as_u64)?;
                    let (project, ask, answer, status) = (
                        text(r, "project"),
                        text(r, "ask"),
                        text(r, "answer"),
                        text(r, "status"),
                    );
                    let stamp = stamps
                        .get(i)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| stamp_of(&project, &ask, &answer, &status));
                    Some(Rule {
                        id,
                        project,
                        ask,
                        answer,
                        status,
                        hits: r.get("hits").and_then(Value::as_u64).unwrap_or(0),
                        answered: r.get("answered").and_then(Value::as_u64).unwrap_or(0),
                        stamp,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn form_ready(project: &str, ask: &str, answer: &str) -> bool {
    let fits = |t: &str| !t.trim().is_empty() && t.chars().count() <= MAX_TEXT;
    !project.trim().is_empty() && fits(ask) && fits(answer)
}

pub fn hits_line(rule: &Rule) -> String {
    match (rule.hits, rule.answered) {
        (0, _) => "No agent question fitted yet.".into(),
        (h, 0) => format!(
            "Fitted {h} {}.",
            if h == 1 { "question" } else { "questions" }
        ),
        (h, a) => format!(
            "Fitted {h} {}, answered {a}.",
            if h == 1 { "question" } else { "questions" }
        ),
    }
}

pub fn build(
    addr: String,
    folder: Option<String>,
    cx: &mut Context<AnswerRulesView>,
) -> AnswerRulesView {
    let rules_doc = cx.new(|cx| doc::build(Document::Rules, addr.clone(), true, cx));
    let project = cx.new(|cx| TextInput::new(cx, "* for all projects, or a full project folder"));
    let ask = cx.new(|cx| TextArea::new(cx, "When the agent asks…").limit(MAX_TEXT));
    let answer = cx.new(|cx| TextArea::new(cx, "Answer with…").limit(MAX_TEXT));
    let watch = vec![
        cx.subscribe(&project, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Changed(_)) {
                keep_form(view, cx);
                cx.notify();
            }
        }),
        cx.subscribe(&ask, |view, _, event: &AreaEvent, cx| {
            on_area(view, event, cx)
        }),
        cx.subscribe(&answer, |view, _, event: &AreaEvent, cx| {
            on_area(view, event, cx)
        }),
    ];
    let mut view = AnswerRulesView {
        addr,
        folder,
        rules_doc,
        rules: None,
        list_error: None,
        form: None,
        project,
        ask,
        answer,
        form_notice: String::new(),
        saving: false,
        remove_ask: None,
        notice: String::new(),
        _watch: watch,
    };
    if let Some(draft) = cx.try_global::<RuleDraft>().map(|d| d.0.clone())
        && draft.form.is_some()
    {
        view.form = draft.form;
        fill(&view, &draft.project, &draft.ask, &draft.answer, cx);
    }
    view
}

pub fn new(addr: String, folder: Option<String>, cx: &mut App) -> Entity<AnswerRulesView> {
    let view = cx.new(|cx| build(addr, folder, cx));
    view.update(cx, reload);
    view
}

fn on_area(view: &mut AnswerRulesView, event: &AreaEvent, cx: &mut Context<AnswerRulesView>) {
    match event {
        AreaEvent::Changed(_) => {
            keep_form(view, cx);
            cx.notify();
        }
        AreaEvent::Save => save_rule(view, cx),
        AreaEvent::Cancel => {}
    }
}

fn values(view: &AnswerRulesView, cx: &App) -> (String, String, String) {
    (
        view.project.read(cx).value().to_string(),
        view.ask.read(cx).value().to_string(),
        view.answer.read(cx).value().to_string(),
    )
}

fn keep_form(view: &AnswerRulesView, cx: &mut App) {
    let (project, ask, answer) = values(view, cx);
    cx.set_global(RuleDraft(FormDraft {
        form: view.form.clone(),
        project,
        ask,
        answer,
    }));
}

fn fill(view: &AnswerRulesView, project: &str, ask: &str, answer: &str, cx: &mut App) {
    let (project, ask, answer) = (project.to_string(), ask.to_string(), answer.to_string());
    view.project
        .update(cx, |input, cx| input.set_value(project, cx));
    view.ask.update(cx, |area, cx| area.set_value(ask, cx));
    view.answer
        .update(cx, |area, cx| area.set_value(answer, cx));
}

pub fn reload(view: &mut AnswerRulesView, cx: &mut Context<AnswerRulesView>) {
    view.rules_doc
        .update(cx, |doc, cx| doc::load(doc, false, cx));
    load_rules(view, cx);
}

pub fn load_rules(view: &mut AnswerRulesView, cx: &mut Context<AnswerRulesView>) {
    let addr = view.addr.clone();
    call(
        &addr,
        "GET",
        "/answer-rules".into(),
        None,
        cx,
        |view, result, _| match result {
            Ok(value) => {
                view.rules = Some(parse_rules(&value));
                view.list_error = None;
            }
            Err(error) => {
                view.list_error = Some(format!("The answer rules could not be read: {error}"))
            }
        },
    );
}

pub fn open_form(
    view: &mut AnswerRulesView,
    rule: Option<&Rule>,
    cx: &mut Context<AnswerRulesView>,
) {
    if view.form.is_some() {
        view.form_notice = "Save or cancel the current rule draft before opening another.".into();
        cx.notify();
        return;
    }
    let (form, project, ask, answer) = match rule {
        Some(rule) => (
            Form {
                id: Some(rule.id),
                status: rule.status.clone(),
                expected: Some(rule.stamp.clone()),
            },
            rule.project.clone(),
            rule.ask.clone(),
            rule.answer.clone(),
        ),
        None => (
            Form {
                id: None,
                status: "trial".into(),
                expected: None,
            },
            view.folder
                .clone()
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| "*".into()),
            String::new(),
            String::new(),
        ),
    };
    view.form = Some(form);
    view.form_notice = String::new();
    fill(view, &project, &ask, &answer, cx);
    keep_form(view, cx);
    cx.notify();
}

pub fn close_form(view: &mut AnswerRulesView, cx: &mut Context<AnswerRulesView>) {
    view.form = None;
    view.form_notice = String::new();
    fill(view, "", "", "", cx);
    cx.set_global(RuleDraft::default());
    cx.notify();
}

pub fn set_status(view: &mut AnswerRulesView, status: &str, cx: &mut Context<AnswerRulesView>) {
    if let Some(form) = view.form.as_mut() {
        form.status = status.to_string();
    }
    keep_form(view, cx);
    cx.notify();
}

pub fn can_save(view: &AnswerRulesView, cx: &App) -> bool {
    let (project, ask, answer) = values(view, cx);
    view.form.is_some() && !view.saving && form_ready(&project, &ask, &answer)
}

pub fn save_rule(view: &mut AnswerRulesView, cx: &mut Context<AnswerRulesView>) {
    if !can_save(view, cx) {
        return;
    }
    let Some(form) = view.form.clone() else {
        return;
    };
    let (project, ask, answer) = values(view, cx);
    let body = json!({"id": form.id, "project": project, "ask": ask, "answer": answer, "status": form.status, "expected": form.expected});
    view.saving = true;
    view.form_notice = "Saving…".into();
    let sent = (project, ask, answer, form.status.clone());
    let addr = view.addr.clone();
    call(
        &addr,
        "POST",
        "/answer-rules".into(),
        Some(body),
        cx,
        move |view, result, cx| on_rule_saved(view, result, sent, cx),
    );
    cx.notify();
}

pub fn on_rule_saved(
    view: &mut AnswerRulesView,
    result: Result<Value, String>,
    sent: (String, String, String, String),
    cx: &mut Context<AnswerRulesView>,
) {
    view.saving = false;
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            view.form_notice = doc::kept_line(&error);
            return;
        }
    };
    let (project, ask, answer) = values(view, cx);
    let status = view
        .form
        .as_ref()
        .map(|f| f.status.clone())
        .unwrap_or_default();
    let changed = (project, ask, answer, status) != sent;
    let saved = value.get("rule").cloned().unwrap_or(Value::Null);
    if changed && let Some(form) = view.form.as_mut() {
        form.id = saved.get("id").and_then(Value::as_u64).or(form.id);
        form.expected = value
            .get("stamp")
            .and_then(Value::as_str)
            .map(str::to_string);
        view.form_notice = "Saved the submitted rule. Your newer edits are still unsaved.".into();
        keep_form(view, cx);
    } else {
        close_form(view, cx);
        view.notice = "Rule saved.".into();
    }
    load_rules(view, cx);
}

pub fn remove_rule(view: &mut AnswerRulesView, rule: &Rule, cx: &mut Context<AnswerRulesView>) {
    view.remove_ask = None;
    let id = rule.id;
    let addr = view.addr.clone();
    call(
        &addr,
        "POST",
        format!("/answer-rules/{id}/remove"),
        Some(json!({"expected": rule.stamp})),
        cx,
        move |view, result, cx| {
            view.notice = match result {
                Ok(_) => format!("Rule {id} removed."),
                Err(error) => error,
            };
            load_rules(view, cx);
        },
    );
    cx.notify();
}

pub fn on_event(
    view: &mut AnswerRulesView,
    msg: &Value,
    cx: &mut Context<AnswerRulesView>,
) -> bool {
    let used = view
        .rules_doc
        .update(cx, |doc, cx| doc::on_event(doc, msg, cx));
    match msg.get("type").and_then(Value::as_str) {
        Some("answer_rules_changed") => {
            load_rules(view, cx);
            true
        }
        Some("hello") => {
            load_rules(view, cx);
            true
        }
        _ => used,
    }
}

pub fn feed(view: &Entity<AnswerRulesView>, msg: &Value, cx: &mut App) -> bool {
    view.update(cx, |view, cx| on_event(view, msg, cx))
}

fn small(_t: &Theme, text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Div {
    div()
        .w_full()
        .text_size(px(11.5))
        .text_color(color)
        .child(text.into())
}

fn heading(t: &Theme, text: &'static str) -> gpui::Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.bright)
        .child(text)
}

fn rule_row(
    view: &AnswerRulesView,
    t: &Theme,
    rule: &Rule,
    cx: &mut Context<AnswerRulesView>,
) -> AnyElement {
    let color = match rule.status.as_str() {
        "live" => t.green,
        "trial" | "offer" => t.yellow,
        _ => t.dim,
    };
    let id = rule.id;
    let edit_rule = rule.clone();
    let remove = rule.clone();
    let asking = view.remove_ask == Some(id);
    div()
        .id(SharedString::from(format!("answer-rule-{id}")))
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(chip(status_word(&rule.status), color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(t.muted)
                        .child(project_label(&rule.project)),
                )
                .child(
                    button(
                        t,
                        SharedString::from(format!("answer-rule-edit-{id}")),
                        "Edit",
                        ButtonKind::Ghost,
                    )
                    .on_click(
                        cx.listener(move |view, _, _, cx| open_form(view, Some(&edit_rule), cx)),
                    ),
                )
                .child(
                    button(
                        t,
                        SharedString::from(format!("answer-rule-remove-{id}")),
                        "Remove",
                        ButtonKind::Ghost,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.remove_ask = Some(id);
                        cx.notify();
                    })),
                ),
        )
        .child(small(
            t,
            format!("When the agent asks: {}", rule.ask),
            t.foreground,
        ))
        .child(small(t, format!("Answer: {}", rule.answer), t.foreground))
        .child(small(t, hits_line(rule), t.dim))
        .when(asking, |el| {
            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(small(t, format!("Remove rule {id}?"), t.red).w_auto())
                    .child(
                        button(
                            t,
                            SharedString::from(format!("answer-rule-keep-{id}")),
                            "Keep",
                            ButtonKind::Secondary,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.remove_ask = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        button(
                            t,
                            SharedString::from(format!("answer-rule-yes-{id}")),
                            "Remove rule",
                            ButtonKind::Danger,
                        )
                        .on_click(
                            cx.listener(move |view, _, _, cx| remove_rule(view, &remove, cx)),
                        ),
                    ),
            )
        })
        .into_any_element()
}

fn field(t: &Theme, label: &'static str, body: impl IntoElement, height: Option<f32>) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(small(t, label, t.dim))
        .child(
            div()
                .w_full()
                .when_some(height, |el, h| el.h(px(h)))
                .child(body),
        )
}

fn form_box(
    view: &AnswerRulesView,
    t: &Theme,
    form: &Form,
    cx: &mut Context<AnswerRulesView>,
) -> AnyElement {
    let ready = can_save(view, cx);
    let saving = view.saving;
    let title = match form.id {
        Some(id) => format!("Edit rule {id}"),
        None => "Add an agent-answer rule".into(),
    };
    let mut statuses = div().flex().flex_wrap().gap(px(6.));
    for status in STATUSES {
        let on = form.status == status;
        let label = if on {
            format!("✓ {}", status_label(status))
        } else {
            status_label(status).to_string()
        };
        statuses = statuses.child(
            disabled(
                button(
                    t,
                    SharedString::from(format!("answer-rule-status-{status}")),
                    label,
                    if on {
                        ButtonKind::Primary
                    } else {
                        ButtonKind::Secondary
                    },
                ),
                saving,
            )
            .when(!saving, |el| {
                el.on_click(cx.listener(move |view, _, _, cx| set_status(view, status, cx)))
            }),
        );
    }
    div()
        .id("answer-rule-form")
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.accent)
        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(title))
        .child(field(t, "Project folder, or * for all projects", view.project.clone(), None))
        .child(field(t, "When the agent asks (up to 400 characters)", view.ask.clone(), Some(76.)))
        .child(field(t, "Answer (up to 400 characters)", view.answer.clone(), Some(76.)))
        .child(statuses)
        .child(small(t, "New rules start in Trial. A Trial rule only counts the questions it fits and does not answer. A Live rule gives Nixa its answer when a question fits, but not when the question names a git step, a push, a deploy, a delete or another risky step. A rule that names a push, deploy, publish, production or destructive step is still saved, and Nixa records it as a decision.", t.dim))
        .when(!view.form_notice.is_empty(), |el| el.child(small(t, view.form_notice.clone(), t.muted)))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(disabled(button(t, "answer-rule-cancel", "Cancel", ButtonKind::Secondary), saving).when(!saving, |el| el.on_click(cx.listener(|view, _, _, cx| close_form(view, cx)))))
                .child(disabled(button(t, "answer-rule-save", if saving { "Saving…" } else { "Save rule" }, ButtonKind::Primary), !ready).when(ready, |el| el.on_click(cx.listener(|view, _, _, cx| save_rule(view, cx))))),
        )
        .into_any_element()
}

fn rules_section(
    view: &AnswerRulesView,
    t: &Theme,
    cx: &mut Context<AnswerRulesView>,
) -> AnyElement {
    let mut list = div()
        .id("answer-rules-list")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.));
    match (&view.list_error, &view.rules) {
        (Some(error), _) => list = list.child(small(t, error.clone(), t.red)),
        (None, None) => list = list.child(small(t, "Loading the answer rules…", t.dim)),
        (None, Some(rules)) if rules.is_empty() => {
            list = list.child(small(
                t,
                "No answer rules yet. Add one when an agent keeps asking the same question.",
                t.dim,
            ))
        }
        (None, Some(rules)) => {
            for rule in rules {
                list = list.child(rule_row(view, t, rule, cx));
            }
        }
    }
    div()
        .id("answer-rules")
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(heading(t, "Answer rules"))
                .child(button(t, "answer-rule-add", "Add rule", ButtonKind::Secondary).on_click(cx.listener(|view, _, _, cx| open_form(view, None, cx)))),
        )
        .child(small(t, "When an agent asks a question that fits a rule, Nixa can answer it with the rule's answer. A rule fits when every clear word of its question (3 letters or more) is in the agent's question, those words are at least half of the agent's question, and the folder is that project or below it. A rule needs at least 2 clear words. The rule with the most exact folder wins. A question about deleting, removing, committing or another risky step is never answered by a rule.", t.dim))
        .when(view.form.is_none() && !view.form_notice.is_empty(), |el| el.child(small(t, view.form_notice.clone(), t.muted)))
        .when_some(view.form.clone(), |el, form| el.child(form_box(view, t, &form, cx)))
        .when(!view.notice.is_empty(), |el| el.child(small(t, view.notice.clone(), t.muted)))
        .child(list)
        .into_any_element()
}

impl Render for AnswerRulesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .id("answer-rules-page")
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.rules_doc.clone())
            .child(small(&t, "Nixa answers agent questions, permissions and plans by itself.", t.dim))
            .child(rules_section(self, &t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_parse_with_stamps() {
        let value = json!({"rules": [{"id": 2, "project": "*", "ask": "Run tests?", "answer": "Yes", "status": "live", "hits": 3, "answered": 1}], "stamps": ["S"]});
        let rules = parse_rules(&value);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].stamp, "S");
        assert_eq!(hits_line(&rules[0]), "Fitted 3 questions, answered 1.");
        let bare = parse_rules(
            &json!({"rules": [{"id": 1, "project": "/p", "ask": "a", "answer": "b", "status": "trial"}]}),
        );
        assert_eq!(bare[0].stamp, r#"["/p","a","b","trial"]"#);
        assert_eq!(hits_line(&bare[0]), "No agent question fitted yet.");
        assert!(parse_rules(&json!({"rules": [{"project": "x"}]})).is_empty());
    }

    #[test]
    fn labels_and_form_checks() {
        assert_eq!(project_label("*"), "All projects");
        assert_eq!(status_label("trial"), "Trial (observe only)");
        assert_eq!(status_word("live"), "Live");
        assert!(form_ready("*", "a", "b"));
        assert!(!form_ready(" ", "a", "b"));
        assert!(!form_ready("*", "", "b"));
        assert!(!form_ready("*", "a", &"x".repeat(401)));
    }
}
