//! Settings → Work → Drones part.
//! Displays and controls Nixa's read-only code watchers (`drone-tree.json`).

use super::memory::doc::call;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, SharedString, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::chip::chip;
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use serde_json::{Value, json};

pub const PART: &str = "Drones";
pub const EVERY_CHOICES: [u64; 7] = [1, 2, 5, 10, 15, 30, 60];
pub const REMIND_CHOICES: [u64; 5] = [5, 15, 30, 60, 120];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRow {
    pub text: String,
    pub step: String,
    pub told: bool,
    pub reminded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRow {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub off: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DronesData {
    pub on: bool,
    pub every: u64,
    pub remind_after: u64,
    pub subjects: usize,
    pub error: Option<String>,
    pub findings: Vec<FindingRow>,
    pub steps: Vec<StepRow>,
}

pub struct DronesView {
    pub addr: String,
    pub data: Option<DronesData>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub busy: bool,
}

pub fn parse_data(value: &Value) -> Option<DronesData> {
    let root = value.get("drones").unwrap_or(value);
    if !root.is_object() {
        return None;
    }
    let tree = root.get("tree").unwrap_or(root);
    let on = root
        .get("on")
        .and_then(Value::as_bool)
        .or_else(|| tree.get("on").and_then(Value::as_bool))
        .unwrap_or(true);
    let every = root
        .get("every")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let remind_after = root
        .get("remindAfter")
        .and_then(Value::as_u64)
        .unwrap_or(15);
    let subjects = root
        .get("subjects")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let error = root
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string);
    let findings = root
        .get("findings")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|f| FindingRow {
                    text: f.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
                    step: f.get("step").and_then(Value::as_str).unwrap_or_default().to_string(),
                    told: f.get("told").and_then(Value::as_bool).unwrap_or(false),
                    reminded: f.get("reminded").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    let steps_obj = root
        .get("steps")
        .or_else(|| tree.get("steps"))
        .and_then(Value::as_object);
    let mut steps = Vec::new();
    if let Some(map) = steps_obj {
        for (id, step) in map {
            let kind = step.get("kind").and_then(Value::as_str).unwrap_or("step").to_string();
            let off = step.get("off").and_then(Value::as_bool).unwrap_or(false);
            let label = step.get("label").and_then(Value::as_str).unwrap_or("");
            let text = step.get("text").and_then(Value::as_str).unwrap_or("");
            let summary = if !label.is_empty() {
                label.to_string()
            } else if !text.is_empty() {
                text.to_string()
            } else {
                step.get("fact")
                    .and_then(Value::as_str)
                    .map(|f| format!("check {f}"))
                    .unwrap_or_else(|| kind.clone())
            };
            steps.push(StepRow { id: id.clone(), kind, summary, off });
        }
    }
    Some(DronesData { on, every, remind_after, subjects, error, findings, steps })
}

pub fn build(addr: String) -> DronesView {
    DronesView {
        addr,
        data: None,
        error: None,
        notice: None,
        busy: false,
    }
}

pub fn new(addr: String, cx: &mut App) -> Entity<DronesView> {
    let view = cx.new(|_| build(addr));
    view.update(cx, reload);
    view
}

pub fn reload(view: &mut DronesView, cx: &mut Context<DronesView>) {
    let addr = view.addr.clone();
    call(
        &addr,
        "GET",
        "/drones".into(),
        None,
        cx,
        |view, result, _| match result {
            Ok(value) => {
                view.data = parse_data(&value);
                view.error = None;
            }
            Err(error) => {
                view.error = Some(format!("Drones could not be read: {error}"));
            }
        },
    );
}

pub fn format_notice(action: &str, text: &str) -> String {
    if matches!(action, "run" | "dry_run")
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
        && let Some(walked) = parsed.get("walked").and_then(Value::as_u64)
    {
        let quiet = parsed.get("quiet").and_then(Value::as_u64).unwrap_or(0);
        let found = parsed
            .get("results")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let label = if action == "dry_run" { "Dry run" } else { "Run" };
        let noun = if found == 1 { "finding" } else { "findings" };
        return format!("{label}: walked {walked}, {found} {noun}, {quiet} quiet.");
    }
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text)
        .to_string()
}

pub fn feed(view: &Entity<DronesView>, msg: &Value, cx: &mut App) {
    let kind = msg.get("type").and_then(Value::as_str);
    if kind == Some("hello") {
        view.update(cx, |v, cx| {
            if v.data.is_none() {
                reload(v, cx);
            }
        });
        return;
    }
    if kind != Some("drones_changed") {
        return;
    }
    if let Some(parsed) = parse_data(msg) {
        view.update(cx, |v, cx| {
            v.data = Some(parsed);
            v.error = None;
            cx.notify();
        });
    } else {
        view.update(cx, reload);
    }
}

fn post_action(
    view: &mut DronesView,
    action: &str,
    mut body: serde_json::Map<String, Value>,
    cx: &mut Context<DronesView>,
) {
    if view.busy {
        return;
    }
    let addr = view.addr.clone();
    body.insert("who".into(), json!("Nixa settings"));
    view.busy = true;
    view.error = None;
    cx.notify();
    let path = format!("/drones/{action}");
    let label = action.to_string();
    call(
        &addr,
        "POST",
        path,
        Some(Value::Object(body)),
        cx,
        move |view, result, cx| {
            view.busy = false;
            match result {
                Ok(value) => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        view.notice = Some(format_notice(&label, text));
                    }
                    reload(view, cx);
                }
                Err(error) => {
                    view.error = Some(error);
                }
            }
        },
    );
}

fn heading(t: &Theme, text: &str) -> gpui::Div {
    div()
        .flex_none()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.foreground)
        .child(text.to_string())
}

fn small(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Div {
    div()
        .w_full()
        .flex_none()
        .text_size(px(11.5))
        .text_color(color)
        .child(text.into())
}

fn timing_row(
    t: &Theme,
    prefix: &'static str,
    choices: &[u64],
    current: u64,
    field: &'static str,
    busy: bool,
    cx: &mut Context<DronesView>,
) -> AnyElement {
    let mut row = div().flex_none().flex().flex_wrap().gap(px(6.));
    for &minutes in choices {
        let active = current == minutes;
        let kind = if active { ButtonKind::Primary } else { ButtonKind::Secondary };
        row = row.child(
            disabled(
                button(
                    t,
                    SharedString::from(format!("{prefix}-{minutes}")),
                    format!("{minutes} min"),
                    kind,
                ),
                busy,
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(move |view, _, _, cx| {
                    let mut body = serde_json::Map::new();
                    body.insert(field.into(), json!(minutes));
                    post_action(view, "timing", body, cx);
                }))
            }),
        );
    }
    row.into_any_element()
}

fn finding_row(t: &Theme, f: &FindingRow) -> AnyElement {
    let badge = if f.reminded {
        chip("reminded", t.yellow)
    } else if f.told {
        chip("reported", t.accent)
    } else {
        chip("new", t.green)
    };
    div()
        .w_full()
        .flex_none()
        .p(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .bg(t.chrome)
        .flex()
        .items_start()
        .gap(px(8.))
        .child(badge)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(12.)).text_color(t.foreground).child(f.text.clone()))
                .when(!f.step.is_empty(), |el| el.child(small(format!("step: {}", f.step), t.dim))),
        )
        .into_any_element()
}

fn step_row(t: &Theme, step: &StepRow, busy: bool, cx: &mut Context<DronesView>) -> AnyElement {
    let id = step.id.clone();
    let next_on = step.off;
    let action = if next_on { "on" } else { "off" };
    let btn_label = if step.off { "Off" } else { "On" };
    let btn_kind = if step.off { ButtonKind::Secondary } else { ButtonKind::Primary };
    div()
        .w_full()
        .flex_none()
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .bg(t.chrome)
        .flex()
        .items_center()
        .gap(px(8.))
        .child(chip(step.kind.clone(), if step.off { t.dim } else { t.accent }))
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
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if step.off { t.dim } else { t.bright })
                        .child(step.id.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.dim)
                        .truncate()
                        .child(step.summary.clone()),
                ),
        )
        .child(
            disabled(
                button(t, SharedString::from(format!("drone-step-{}", step.id)), btn_label, btn_kind),
                busy,
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(move |view, _, _, cx| {
                    let mut body = serde_json::Map::new();
                    body.insert("id".into(), json!(id));
                    post_action(view, action, body, cx);
                }))
            }),
        )
        .into_any_element()
}

impl Render for DronesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let busy = self.busy;
        let mut page = div()
            .id("drones-page")
            .debug_selector(|| "drones-page".into())
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.))
            .pb(px(24.))
            .child(small(
                "Drones are read-only code watchers. Every few minutes they walk drone-tree.json over every open task and live terminal (skipping locked spaces) and report stalls to Nixa. They never edit tasks, claims or terminals on their own.",
                t.dim,
            ));

        if let Some(error) = &self.error {
            page = page.child(small(error.clone(), t.red));
        }
        if let Some(notice) = &self.notice {
            page = page.child(small(notice.clone(), t.accent));
        }

        let Some(data) = self.data.clone() else {
            return page.child(small("Loading drones…", t.dim));
        };

        if let Some(file_err) = &data.error {
            page = page.child(small(file_err.clone(), t.red));
        }

        let on = data.on;
        let master = toggle_row(
            &t,
            "drones-master-on",
            "Drones enabled",
            Some(
                format!(
                    "Check every {} min · remind once after {} min · {} subject{} watched.",
                    data.every,
                    data.remind_after,
                    data.subjects,
                    if data.subjects == 1 { "" } else { "s" }
                )
                .into(),
            ),
            on,
        )
        .debug_selector(|| "drones-master-on".into())
        .flex_none()
        .when(busy, |el| el.opacity(0.6))
        .when(!busy, |el| {
            el.on_click(cx.listener(move |view, _, _, cx| {
                post_action(view, if on { "off" } else { "on" }, serde_json::Map::new(), cx);
            }))
        });

        let actions = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(
                disabled(button(&t, "drones-run-now", "Run now", ButtonKind::Primary), busy)
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|view, _, _, cx| {
                            post_action(view, "run", serde_json::Map::new(), cx)
                        }))
                    }),
            )
            .child(
                disabled(button(&t, "drones-dry-run", "Dry run", ButtonKind::Secondary), busy)
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|view, _, _, cx| {
                            post_action(view, "dry_run", serde_json::Map::new(), cx)
                        }))
                    }),
            )
            .child(
                disabled(button(&t, "drones-undo", "Undo edit", ButtonKind::Secondary), busy)
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|view, _, _, cx| {
                            post_action(view, "undo", serde_json::Map::new(), cx)
                        }))
                    }),
            )
            .child(
                disabled(button(&t, "drones-refresh", "Refresh", ButtonKind::Secondary), busy)
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|view, _, _, cx| reload(view, cx)))
                    }),
            );

        let mut findings_box = div().w_full().flex_none().flex().flex_col().gap(px(6.));
        if data.findings.is_empty() {
            findings_box = findings_box.child(small("All clear — no open findings.", t.dim));
        } else {
            for f in &data.findings {
                findings_box = findings_box.child(finding_row(&t, f));
            }
        }

        let mut steps_box = div().w_full().flex_none().flex().flex_col().gap(px(4.));
        for step in &data.steps {
            steps_box = steps_box.child(step_row(&t, step, busy, cx));
        }

        page.child(master)
            .child(actions)
            .child(heading(&t, "Check every").mt(px(4.)))
            .child(timing_row(&t, "drone-every", &EVERY_CHOICES, data.every, "every", busy, cx))
            .child(heading(&t, "Remind once after").mt(px(4.)))
            .child(timing_row(&t, "drone-remind", &REMIND_CHOICES, data.remind_after, "remind", busy, cx))
            .child(heading(&t, &format!("Open findings ({})", data.findings.len())).mt(px(6.)))
            .child(findings_box)
            .child(heading(&t, &format!("Tree steps ({})", data.steps.len())).mt(px(6.)))
            .child(steps_box)
            .child(
                small(
                    "Edit steps with `nixa drones` or ask Nixa (`nixa_drones`). Changes are validated, saved to drone-tree.json and logged in drone-edits.jsonl.",
                    t.dim,
                )
                .mt(px(4.)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drones_payload() {
        let payload = json!({
            "drones": {
                "on": true,
                "every": 5,
                "remindAfter": 20,
                "subjects": 4,
                "findings": [
                    {"text": "Task #1 stuck", "step": "b-inbox-report", "told": true, "reminded": false}
                ],
                "tree": {
                    "steps": {
                        "subject": {"kind": "switch", "label": "Task or terminal", "fact": "subject"},
                        "d-idle": {"kind": "all", "label": "Idle terminal", "off": true}
                    }
                }
            }
        });
        let parsed = parse_data(&payload).unwrap();
        assert!(parsed.on);
        assert_eq!(parsed.every, 5);
        assert_eq!(parsed.remind_after, 20);
        assert_eq!(parsed.subjects, 4);
        assert_eq!(parsed.findings.len(), 1);
        assert_eq!(parsed.steps.len(), 2);
        assert!(parsed.steps.iter().any(|s| s.id == "d-idle" && s.off));
    }

    #[test]
    fn parses_drones_changed_snapshot_payload() {
        let payload = json!({
            "type": "drones_changed",
            "drones": {
                "on": false,
                "every": 10,
                "remindAfter": 30,
                "subjects": 2,
                "error": "Bad step in drone-tree.json",
                "findings": [
                    {"text": "Terminal m2 idle", "step": "d-idle-report", "told": true, "reminded": true}
                ],
                "steps": {
                    "check": {"kind": "if", "fact": "idle_min"}
                }
            }
        });
        let parsed = parse_data(&payload).unwrap();
        assert!(!parsed.on);
        assert_eq!(parsed.every, 10);
        assert_eq!(parsed.remind_after, 30);
        assert_eq!(parsed.subjects, 2);
        assert_eq!(parsed.error.as_deref(), Some("Bad step in drone-tree.json"));
        assert!(parsed.findings[0].reminded);
        assert_eq!(parsed.steps[0].summary, "check idle_min");
    }

    #[test]
    fn formats_run_and_dry_run_notices_from_json() {
        let raw = serde_json::to_string_pretty(&json!({
            "walked": 4,
            "quiet": 3,
            "results": [{"subject": "task #1", "end": "report"}]
        }))
        .unwrap();
        assert_eq!(format_notice("run", &raw), "Run: walked 4, 1 finding, 3 quiet.");
        assert_eq!(format_notice("dry_run", &raw), "Dry run: walked 4, 1 finding, 3 quiet.");
        assert_eq!(
            format_notice("on", "Drones turned on. Edit saved and logged."),
            "Drones turned on. Edit saved and logged."
        );
    }
}
