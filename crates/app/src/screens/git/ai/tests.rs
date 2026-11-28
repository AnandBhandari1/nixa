use super::menu::{self, Item};
use super::model::{self, Deps, Kind, Request};
use super::view::{AiResultEvent, AiResultView, Mode};
use super::voice::{self, Level};
use super::{GitAiCommand, request_for};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use nixa_git::ai::{AiError, AiSettings, CommitText, ErrorKind, Found};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const REVIEW: &str = "**Summary**\nAdds a check.\n\n**Findings**\n- Bug: `x = 1` assigns\n- Nit | name\n\n**Verdict**\nNeeds changes";

fn found(id: &str, label: &str, installed: bool) -> Found {
    Found { id: id.into(), label: label.into(), installed, path: format!("/fake/{id}") }
}

struct Fake {
    deps: Deps,
    asks: Arc<AtomicUsize>,
    picks: Arc<Mutex<Vec<String>>>,
}

fn fake(answer: impl Fn(usize, &nixa_git::ai::Opts) -> Result<String, AiError> + Send + Sync + 'static) -> Fake {
    let asks = Arc::new(AtomicUsize::new(0));
    let picks = Arc::new(Mutex::new(Vec::new()));
    let count = asks.clone();
    let saved = picks.clone();
    let deps = Deps {
        ask: Arc::new(move |_, _, opts| answer(count.fetch_add(1, Ordering::SeqCst), opts)),
        load: Arc::new(|| {
            let mut s = AiSettings::default();
            s.models.insert("claude".into(), "sonnet".into());
            s
        }),
        detect: Arc::new(|| vec![found("claude", "Claude Code", true), found("codex", "Codex CLI", true), found("omp", "Oh My Pi", false)]),
        pick: Arc::new(move |id| {
            saved.lock().unwrap().push(id.to_string());
            Ok(())
        }),
    };
    Fake { deps, asks, picks }
}

fn open(cx: &mut TestAppContext, request: Request, deps: Deps) -> (Entity<AiResultView>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (view, vcx) = cx.add_window_view(move |_, cx| AiResultView::new(Mode::Overlay, request, deps, cx));
    (view, vcx)
}

fn settle(vcx: &mut VisualTestContext, view: &Entity<AiResultView>, done: impl Fn(&AiResultView) -> bool) {
    let began = Instant::now();
    loop {
        vcx.run_until_parked();
        if view.read_with(vcx, |v, _| done(v)) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "view never settled");
        std::thread::sleep(Duration::from_millis(5));
        vcx.executor().advance_clock(model::POLL);
    }
}

fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.run_until_parked();
    vcx.debug_bounds(selector).is_some()
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn repo() -> Request {
    Request::review_staged("/tmp/nixa-git-ai-test-repo")
}

#[test]
fn request_texts() {
    assert_eq!(model::target_text(&repo()), "Staged changes");
    assert_eq!(model::target_text(&Request::review_commit("/r", "0123456789")), "Commit 01234567");
    assert_eq!(model::target_text(&Request::explain("/r", "")), "Last commit");
    assert_eq!(model::target_text(&Request::conflict("/r", "a.rs", None)), "Conflict in a.rs");
    assert_eq!(model::target_text(&Request::conflict("/r", "", None)), "Conflict");
    assert_eq!(model::heading(Some(Kind::Review)), "AI review");
    assert_eq!(model::heading(Some(Kind::Explain)), "AI explains a commit");
    assert_eq!(model::heading(Some(Kind::Conflict)), "AI explains a conflict");
    assert_eq!(model::waiting_text("", false), "Asking the AI… This can take a minute.");
    assert_eq!(model::waiting_text("Codex CLI", true), "Stopping…");
    assert_eq!(model::chip_text("Claude Code", "opus"), "Claude Code · opus");
    assert_eq!(model::chip_text("Claude Code", ""), "Claude Code");
    assert_eq!(model::took_secs(200), 1);
    assert_eq!(model::took_secs(2600), 3);
    assert_eq!(request_for(GitAiCommand::ExplainLastCommit, "/r".into()), Request::explain("/r", "HEAD"));
    assert_eq!(request_for(GitAiCommand::ReviewLastCommit, "/r".into()).sha.as_deref(), Some("HEAD"));
}

#[test]
fn show_specs() {
    assert_eq!(model::parse_show("review:/tmp/r"), Some(Request::review_staged("/tmp/r")));
    assert_eq!(model::parse_show("review@abc:/tmp/r"), Some(Request::review_commit("/tmp/r", "abc")));
    assert_eq!(model::parse_show("explain:/tmp/r"), Some(Request::explain("/tmp/r", "HEAD")));
    assert_eq!(model::parse_show("conflict@a.rs:/tmp/r"), Some(Request::conflict("/tmp/r", "a.rs", None)));
    assert_eq!(model::parse_show("conflict:/tmp/r"), None);
    assert_eq!(model::parse_show("review:"), None);
    assert_eq!(model::parse_show("other:/r"), None);
}

#[test]
fn model_lines() {
    let mut s = AiSettings::default();
    assert_eq!(model::model_text(&s, "claude"), "Model: the agent's default");
    assert_eq!(model::model_text(&s, "ollama"), "Model: first one in ollama list");
    s.models.insert("codex".into(), "gpt-5".into());
    assert_eq!(model::model_text(&s, "codex"), "Model: gpt-5");
    s.agent = "codex".into();
    assert_eq!(model::agent_label(&s), ("Codex CLI".to_string(), "gpt-5".to_string()));
    s.agent = "mystery".into();
    assert_eq!(model::agent_label(&s).0, "mystery");
}

#[test]
fn menu_items_and_keys() {
    let list = vec![found("claude", "Claude Code", true)];
    assert_eq!(menu::items(Some(&list)), vec![Item::Agent("claude".into()), Item::Settings]);
    assert_eq!(menu::items(None), vec![Item::Settings]);
    assert_eq!(menu::step(0, 3, -1), 2);
    assert_eq!(menu::step(2, 3, 1), 0);
    assert_eq!(menu::step(0, 0, 1), 0);
}

#[test]
fn git_voice_lines() {
    assert!(voice::allowed(true, false));
    assert!(!voice::allowed(false, false));
    assert!(!voice::allowed(true, true));
    assert_eq!(voice::message("Push done"), json!({"text": "Push done", "agent": "nixa", "kind": "git"}));
    let ok: Result<CommitText, AiError> = Ok(CommitText { summary: "fix: x".into(), body: String::new(), agent: "claude".into() });
    let r = voice::commit_result(&ok);
    assert_eq!((r.title.as_str(), r.body.as_str(), r.level, r.spoken.as_str()), ("Commit message ready", "fix: x", Level::Success, "Commit message ready"));
    let stop = voice::commit_result(&Err(AiError::new(ErrorKind::Cancelled, "Stopped", None)));
    assert_eq!((stop.title.as_str(), stop.body.as_str()), ("Stopped", "No commit message was written."));
    let gone = voice::commit_result(&Err(AiError::new(ErrorKind::Missing, "Codex CLI is not installed", Some("codex"))));
    assert_eq!(gone.body, format!("Codex CLI is not installed\n{}", voice::MISSING_TIP));
    assert_eq!(gone.spoken, "Commit message failed");
    assert_eq!(gone.level, Level::Error);
    let wait = voice::wait_for(Some("Push"));
    assert_eq!((wait.title.as_str(), wait.spoken.as_str()), ("Wait for the push to finish", "Wait a moment"));
    assert_eq!(voice::wait_for(None).title, "Wait for the current git work to finish");
    assert_eq!(voice::pick_profile("push").title, "Push not done");
    assert_eq!(voice::pick_profile("x").spoken, "Pick a profile first");
    assert_eq!(voice::fix_conflicts().spoken, "Fix the conflicts first");
    assert_eq!(voice::discarded_all(1).body, "1 file");
    assert_eq!(voice::discarded_all(3).body, "3 files");
    assert_eq!(voice::side_taken("a.rs", false).spoken, "Took incoming");
    assert_eq!(voice::resolved("a.rs is resolved").spoken, "Marked resolved");
    assert!(voice::open_repo_first("nixa").body.contains("Open nixa in the panel"));
}

#[gpui::test]
fn review_shows_verdict_and_copies(cx: &mut TestAppContext) {
    let f = fake(|_, _| Ok(REVIEW.to_string()));
    let (view, vcx) = open(cx, repo(), f.deps.clone());
    settle(vcx, &view, |v| !v.busy);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.text, REVIEW);
        assert_eq!(v.agent, "Claude Code");
        assert_eq!(v.model, "sonnet");
        assert!(v.took >= 1);
        assert_eq!(v.error, None);
    });
    assert!(drawn(vcx, "ai-result-verdict"));
    assert!(drawn(vcx, "ai-result-agent"));
    click(vcx, "ai-result-copy");
    assert!(view.read_with(vcx, |v, _| v.copied));
    assert_eq!(vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some(REVIEW));
    assert_eq!(f.asks.load(Ordering::SeqCst), 1);
}

#[gpui::test]
fn stop_then_retry(cx: &mut TestAppContext) {
    let f = fake(|n, opts| {
        if n > 0 {
            return Ok("Plain answer.".into());
        }
        let began = Instant::now();
        while !opts.cancel.load(Ordering::SeqCst) {
            if began.elapsed() > Duration::from_secs(10) {
                return Ok("never stopped".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(AiError::new(ErrorKind::Cancelled, "Stopped", Some("claude")))
    });
    let (view, vcx) = open(cx, repo(), f.deps.clone());
    settle(vcx, &view, |v| !v.agent.is_empty());
    assert!(view.read_with(vcx, |v, _| v.busy));
    assert!(drawn(vcx, "ai-result-waiting"));
    assert!(!drawn(vcx, "ai-result-retry"));
    click(vcx, "ai-result-stop");
    assert!(view.read_with(vcx, |v, _| v.stopping));
    settle(vcx, &view, |v| !v.busy);
    assert_eq!(view.read_with(vcx, |v, _| v.error.clone()).as_deref(), Some(model::STOPPED));
    assert!(drawn(vcx, "ai-result-error"));
    click(vcx, "ai-result-retry");
    settle(vcx, &view, |v| !v.busy);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.text, "Plain answer.");
        assert_eq!(v.error, None);
        assert_eq!(v.run, 2);
    });
    assert!(!drawn(vcx, "ai-result-verdict"));
    assert_eq!(f.asks.load(Ordering::SeqCst), 2);
}

#[gpui::test]
fn missing_agent_says_change_agent(cx: &mut TestAppContext) {
    let f = fake(|_, _| Err(AiError::new(ErrorKind::Missing, "Codex CLI is not installed", Some("codex"))));
    let (view, vcx) = open(cx, repo(), f.deps.clone());
    settle(vcx, &view, |v| !v.busy);
    view.read_with(vcx, |v, _| {
        assert!(v.missing);
        assert_eq!(v.error.as_deref(), Some("Codex CLI is not installed"));
    });
    assert!(!drawn(vcx, "ai-result-copy"));
}

#[gpui::test]
fn no_repo_is_an_error(cx: &mut TestAppContext) {
    let f = fake(|_, _| Ok("x".into()));
    let (view, vcx) = open(cx, Request { kind: Some(Kind::Review), ..Request::default() }, f.deps.clone());
    settle(vcx, &view, |v| !v.busy);
    assert_eq!(view.read_with(vcx, |v, _| v.error.clone()).as_deref(), Some(model::NO_REPO));
    assert_eq!(f.asks.load(Ordering::SeqCst), 0);
}

#[gpui::test]
fn change_agent_picks_from_the_menu(cx: &mut TestAppContext) {
    let f = fake(|_, _| Ok("ok".into()));
    let (view, vcx) = open(cx, repo(), f.deps.clone());
    settle(vcx, &view, |v| !v.busy);
    click(vcx, "ai-result-change");
    let menu = view.read_with(vcx, |v, _| v.menu.clone()).expect("menu open");
    let began = Instant::now();
    while menu.read_with(vcx, |m, _| m.agents.is_none()) {
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
        vcx.executor().advance_clock(model::POLL);
        vcx.run_until_parked();
    }
    menu.read_with(vcx, |m, _| {
        assert_eq!(m.agents.as_ref().map(Vec::len), Some(2));
        assert_eq!(m.at, 0);
    });
    assert!(drawn(vcx, "agent-menu-codex"));
    assert!(drawn(vcx, "agent-menu-settings"));
    click(vcx, "agent-menu-codex");
    settle(vcx, &view, |v| v.menu.is_none());
    assert_eq!(*f.picks.lock().unwrap(), vec!["codex".to_string()]);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.agent, "Codex CLI");
        assert_eq!(v.note.as_deref(), Some("Press Retry to ask Codex CLI."));
    });
    click(vcx, "ai-result-change");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.menu.is_none()));
}

#[gpui::test]
fn close_and_settings_send_events(cx: &mut TestAppContext) {
    let f = fake(|_, _| Ok("ok".into()));
    let (view, vcx) = open(cx, repo(), f.deps.clone());
    let seen: Arc<Mutex<Vec<AiResultEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &AiResultEvent, _| log.lock().unwrap().push(e.clone())).detach();
    });
    settle(vcx, &view, |v| !v.busy);
    click(vcx, "ai-result-agent");
    click(vcx, "ai-result-close");
    assert_eq!(*seen.lock().unwrap(), vec![AiResultEvent::OpenSettings, AiResultEvent::Close]);
}

#[gpui::test]
fn old_runs_are_ignored(cx: &mut TestAppContext) {
    let f = fake(|_, _| Ok("new".into()));
    let (view, vcx) = open(cx, repo(), f.deps.clone());
    settle(vcx, &view, |v| !v.busy);
    view.update(vcx, |v, cx| v.finish(v.run - 1, Ok("old".into()), cx));
    assert_eq!(view.read_with(vcx, |v, _| v.text.clone()), "new");
}
