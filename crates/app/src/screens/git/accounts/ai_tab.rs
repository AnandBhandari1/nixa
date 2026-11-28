use super::kit::{self, Ctx};
use crate::screens::model_pick::{self, Listed, ModelPick, ModelPickEvent};
use crate::screens::settings::parts::memory::text_area::{AreaEvent, TextArea};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Window, div, px};
use nixa_git::ai::agents::ollama_models;
use nixa_git::ai::prompts::{COMMIT_STYLES, format_prefix_rules, parse_prefix_rules, resolve_prefix};
use nixa_git::ai::{AiError, AiSettings, Answer, Found, Opts, SETTINGS_KEY, agent_by_id, model_placeholder};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

pub const MIN_SECS: u64 = 10;
pub const MAX_SECS: u64 = 3600;

#[derive(Clone, Copy)]
pub struct AiDeps {
    pub detect: fn(bool) -> Vec<Found>,
    pub ping: fn(&AiSettings, &Opts) -> Result<Answer, AiError>,
    pub ollama: fn(&str) -> Vec<String>,
    pub models: fn(&str, bool) -> Result<Listed, String>,
}

pub fn real_deps() -> AiDeps {
    AiDeps { detect: nixa_git::ai::detect_agents, ping: nixa_git::ai::ping, ollama: ollama_models, models: model_pick::brain_list }
}

pub struct AiTab {
    pub ctx: Ctx,
    pub deps: AiDeps,
    pub branch: Option<String>,
    pub settings: AiSettings,
    pub found: Vec<Found>,
    pub looking: bool,
    pub ollama: Vec<String>,
    pub save_error: Option<String>,
    pub bad_lines: Vec<usize>,
    pub timeout_error: bool,
    pub testing: Option<nixa_git::ai::Cancel>,
    pub test_result: Option<(bool, String)>,
    pub model: Entity<ModelPick>,
    pub timeout: Entity<TextInput>,
    pub custom: Entity<TextArea>,
    pub prefix: Entity<TextArea>,
    pub review: Entity<TextArea>,
}

pub fn new(ctx: Ctx, deps: AiDeps, repo: Option<PathBuf>, cx: &mut App) -> Entity<AiTab> {
    cx.new(|cx| build(ctx, deps, repo, cx))
}

pub fn build(ctx: Ctx, deps: AiDeps, repo: Option<PathBuf>, cx: &mut Context<AiTab>) -> AiTab {
    let settings = AiSettings::from_value(kit::get_setting(&ctx.db, SETTINGS_KEY).as_ref());
    let model = model_pick::new("ai-model", &settings.agent, settings.model_of(&settings.agent), model_pick::load_of(deps.models), cx);
    let timeout = kit::input("240", &settings.timeout_sec.to_string(), false, cx);
    let custom = cx.new(|cx| TextArea::new(cx, "For example: Start with the ticket number. Use the past tense."));
    let prefix = cx.new(|cx| TextArea::new(cx, "feature/* => [{suffix}]").mono());
    let review = cx.new(|cx| TextArea::new(cx, "For example: Check error handling. Check that new code has tests."));
    let (i1, p1, r1) = (settings.style.instructions.clone(), format_prefix_rules(&settings.style.prefix_rules), settings.review.instructions.clone());
    custom.update(cx, |a, cx| a.set_value(i1, cx));
    prefix.update(cx, |a, cx| a.set_value(p1, cx));
    review.update(cx, |a, cx| a.set_value(r1, cx));
    cx.subscribe(&model, |v, _, e: &ModelPickEvent, cx| {
        let ModelPickEvent::Picked(text) = e;
        let text = text.trim().to_string();
        let agent = v.settings.agent.clone();
        if text.is_empty() {
            v.settings.models.remove(&agent);
        } else {
            v.settings.models.insert(agent, text);
        }
        store(v, cx);
    })
    .detach();
    cx.subscribe(&timeout, |v, input, e: &InputEvent, cx| {
        if let InputEvent::Changed(_) = e {
            let text = input.read(cx).value().trim().to_string();
            match text.parse::<u64>() {
                Ok(n) if (MIN_SECS..=MAX_SECS).contains(&n) => {
                    v.timeout_error = false;
                    v.settings.timeout_sec = n;
                    store(v, cx);
                }
                _ => {
                    v.timeout_error = true;
                    cx.notify();
                }
            }
        }
    })
    .detach();
    cx.subscribe(&custom, |v, area, e: &AreaEvent, cx| {
        if let AreaEvent::Changed(_) = e {
            v.settings.style.instructions = area.read(cx).value().to_string();
            store(v, cx);
        }
    })
    .detach();
    cx.subscribe(&prefix, |v, area, e: &AreaEvent, cx| {
        if let AreaEvent::Changed(_) = e {
            let (rules, bad) = parse_prefix_rules(area.read(cx).value());
            v.bad_lines = bad;
            v.settings.style.prefix_rules = rules;
            store(v, cx);
        }
    })
    .detach();
    cx.subscribe(&review, |v, area, e: &AreaEvent, cx| {
        if let AreaEvent::Changed(_) = e {
            v.settings.review.instructions = area.read(cx).value().to_string();
            store(v, cx);
        }
    })
    .detach();
    let branch = repo.as_deref().and_then(nixa_git::ai::repo::current_branch);
    let mut view = AiTab {
        ctx,
        deps,
        branch,
        settings,
        found: Vec::new(),
        looking: false,
        ollama: Vec::new(),
        save_error: None,
        bad_lines: Vec::new(),
        timeout_error: false,
        testing: None,
        test_result: None,
        model,
        timeout,
        custom,
        prefix,
        review,
    };
    look(&mut view, false, cx);
    view
}

pub fn store(v: &mut AiTab, cx: &mut Context<AiTab>) {
    v.save_error = kit::set_setting(&v.ctx.db, SETTINGS_KEY, &v.settings.to_value())
        .err()
        .map(|e| format!("Could not save the AI settings: {e}"));
    cx.notify();
}

pub fn look(v: &mut AiTab, fresh: bool, cx: &mut Context<AiTab>) {
    if v.looking {
        return;
    }
    v.looking = true;
    let deps = v.deps;
    kit::job(cx, move || (deps.detect)(fresh), |v, found, cx| {
        v.found = found;
        v.looking = false;
        load_ollama(v, cx);
    });
    cx.notify();
}

fn load_ollama(v: &mut AiTab, cx: &mut Context<AiTab>) {
    let Some(f) = v.found.iter().find(|f| f.id == "ollama" && f.installed) else {
        v.ollama.clear();
        return;
    };
    if v.settings.agent != "ollama" {
        return;
    }
    let path = f.path.clone();
    let deps = v.deps;
    kit::job(cx, move || (deps.ollama)(&path), |v, models, cx| {
        v.model.update(cx, |p, cx| p.set_extra(models.clone(), cx));
        v.ollama = models;
    });
}

pub fn pick_agent(v: &mut AiTab, id: &str, cx: &mut Context<AiTab>) {
    v.settings.agent = id.to_string();
    v.test_result = None;
    let model = v.settings.model_of(id).to_string();
    let agent = id.to_string();
    v.model.update(cx, |p, cx| p.set_agent(&agent, &model, cx));
    store(v, cx);
    load_ollama(v, cx);
}

pub fn pick_style(v: &mut AiTab, id: &str, cx: &mut Context<AiTab>) {
    v.settings.style.preset = id.to_string();
    store(v, cx);
}

pub fn test_agent(v: &mut AiTab, cx: &mut Context<AiTab>) {
    if let Some(stop) = v.testing.take() {
        stop.store(true, Ordering::SeqCst);
        v.test_result = Some((false, "Stopped.".into()));
        cx.notify();
        return;
    }
    let stop = nixa_git::ai::cancel();
    v.testing = Some(stop.clone());
    v.test_result = None;
    let settings = v.settings.clone();
    let deps = v.deps;
    let opts = Opts { agent: Some(settings.agent.clone()), model: None, timeout_sec: None, cancel: stop.clone() };
    kit::job(cx, move || (deps.ping)(&settings, &opts), move |v, done, _| {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        v.testing = None;
        v.test_result = Some(match done {
            Ok(a) => {
                let model = if a.model.is_empty() { String::new() } else { format!(" ({})", a.model) };
                (true, format!("{}{model} answered: {}", a.label, a.text.trim()))
            }
            Err(e) => (false, e.message),
        });
    });
    cx.notify();
}

fn area(id: &'static str, el: &Entity<TextArea>, h: f32) -> gpui::Div {
    div().w_full().h(px(h)).flex().flex_col().debug_selector(move || id.into()).child(el.clone())
}

fn agent_list(v: &AiTab, t: &Theme, cx: &mut Context<AiTab>) -> AnyElement {
    if v.found.is_empty() && v.looking {
        return kit::busy_line(t, "ai-looking", "Looking for AI tools…").into_any_element();
    }
    let mut col = div().flex().flex_col();
    for f in &v.found {
        let on = f.id == v.settings.agent;
        let detail = if f.installed { kit::short_path(&f.path, &v.ctx.env.home) } else { "Not installed".to_string() };
        let id = f.id.clone();
        let row = kit::radio(t, format!("ai-agent-{}", f.id), on, f.label.clone(), &detail);
        col = col.child(if f.installed || on {
            row.on_click(cx.listener(move |v, _, _, cx| pick_agent(v, &id, cx))).into_any_element()
        } else {
            row.opacity(0.5).into_any_element()
        });
    }
    if !v.found.iter().any(|f| f.installed) && !v.looking {
        col = col.child(kit::warn_text(t, "No AI tool was found on this computer."));
    }
    col.into_any_element()
}

fn model_block(v: &AiTab, t: &Theme) -> AnyElement {
    let agent = agent_by_id(&v.settings.agent);
    let label = agent.map(|a| a.label).unwrap_or("the agent");
    let first = v.ollama.first().cloned().unwrap_or_default();
    let note = match agent {
        Some(a) if a.needs_model && first.is_empty() => "Default uses the first model in ollama list.".to_string(),
        Some(a) if a.needs_model => format!("Default uses {first}."),
        _ => format!("Default uses what {label} uses by default."),
    };
    let place = model_placeholder(agent, &first);
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(kit::label(t, format!("Model for {label}")))
        .child(v.model.clone())
        .child(kit::hint(t, format!("{place}. {note}")))
        .into_any_element()
}

fn prefix_note(v: &AiTab, t: &Theme) -> AnyElement {
    let mut col = div().flex().flex_col();
    for n in &v.bad_lines {
        col = col.child(kit::warn_text(t, format!("Line {n} needs the form: pattern => prefix")));
    }
    if let Some(branch) = v.branch.as_ref()
        && !v.settings.style.prefix_rules.is_empty()
    {
        col = col.child(match resolve_prefix(&v.settings.style.prefix_rules, Some(branch)) {
            Some(p) => kit::ok_text(t, format!("On {branch}, messages start with \"{p}\".")),
            None => kit::hint(t, format!("No rule matches the branch {branch}.")),
        });
    }
    col.into_any_element()
}

impl Render for AiTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let agents = agent_list(self, &t, cx);
        let model = model_block(self, &t);
        let mut styles = div().flex().flex_col();
        for s in COMMIT_STYLES {
            let id = s.id;
            styles = styles.child(
                kit::radio(&t, format!("ai-style-{id}"), self.settings.style.preset == id, s.label, s.description)
                    .on_click(cx.listener(move |v, _, _, cx| pick_style(v, id, cx))),
            );
        }
        let testing = self.testing.is_some();
        let label = agent_by_id(&self.settings.agent).map(|a| a.label).unwrap_or("the agent");
        kit::scroll_body("ai-tab")
            .child(
                kit::row()
                    .child(kit::label(&t, "AI agent"))
                    .child(div().flex_1())
                    .child(
                        kit::act(&t, "ai-look", "Look again", ButtonKind::Secondary, self.looking)
                            .on_click(cx.listener(|v, _, _, cx| look(v, true, cx))),
                    ),
            )
            .child(agents)
            .child(model)
            .child(kit::label(&t, "Commit message style"))
            .child(styles)
            .when(self.settings.style.preset == "custom", |el| el.child(area("ai-custom", &self.custom, 80.)))
            .child(kit::label(&t, "Branch prefix rules"))
            .child(area("ai-prefix", &self.prefix, 80.))
            .child(kit::hint(
                &t,
                "One rule per line: pattern => prefix. A * matches any text. The prefix can use {branch}, {suffix} (the part after the last /) and {ticket} (like ABC-123).",
            ))
            .child(prefix_note(self, &t))
            .child(kit::label(&t, "Review rules"))
            .child(area("ai-review", &self.review, 80.))
            .child(kit::hint(
                &t,
                "Extra things the AI should check when it reviews your changes. A repo can add its own rules in .nixa/review.md.",
            ))
            .child(kit::label(&t, "Time limit (seconds)"))
            .child(div().w(px(120.)).child(kit::field(&self.timeout, "ai-timeout")))
            .child(if self.timeout_error {
                kit::warn_text(&t, "Use a whole number from 10 to 3600.")
            } else {
                kit::hint(&t, "The AI is stopped after this many seconds.")
            })
            .child(kit::label(&t, "Check the agent"))
            .child(
                kit::row()
                    .child(
                        kit::act(&t, "ai-test", if testing { "Stop" } else { "Test agent" }, ButtonKind::Secondary, false)
                            .on_click(cx.listener(|v, _, _, cx| test_agent(v, cx))),
                    )
                    .child(if testing {
                        kit::busy_line(&t, "ai-test-spin", format!("Asking {label}…")).into_any_element()
                    } else {
                        kit::hint(&t, "Sends a tiny test question.").into_any_element()
                    }),
            )
            .when_some(self.test_result.clone(), |el, (ok, text)| {
                el.child(if ok { kit::ok_text(&t, text) } else { kit::error_text(&t, text) })
            })
            .when_some(self.save_error.clone(), |el, e| el.child(kit::error_text(&t, e)))
    }
}
