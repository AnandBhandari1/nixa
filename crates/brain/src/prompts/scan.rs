use super::ask_match::same_ask;
use super::book::{NewPrompt, Prompt, PromptKind, SCREEN_PREFIX, plain_options};
use super::card::describe;
use super::dialog::{DialogKind, DialogRead, Question, dialog_key, parse_dialog, question_from_dialog, screen_tail};
use super::link::Link;
use super::retry::{ReadFrom, read_with_retry};
use super::screen::{ScreenState, read_screen};
use super::{broadcast, notes};
use crate::app::{App, lock};
use crate::spaces::Space;
use crate::util::{log, now_ms};
use nixa_protocol::{Status, TermInfo};
use regex::Regex;
use std::sync::{Arc, LazyLock};

pub const HOOK_GRACE_MS: u64 = 6000;
pub const SAME_ASK_MS: u64 = 30 * 60 * 1000;
pub const TAIL_LINES: usize = 15;
pub const GONE_MS: u64 = 1000;
const INTERPRETERS: [&str; 9] = ["python3", "python", "node", "bash", "sh", "zsh", "ruby", "deno", "bun"];

static PERMISSION_NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)permission").expect("permission note"));
static PLAN_NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)plan").expect("plan note"));
static APPROVE_PLAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)approve plan").expect("approve plan"));
static TOOL_NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)permission to use ([\w.:-]+)").expect("tool note"));
static WORKING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)esc to interrupt").expect("working"));
static TOOL_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Z][\w-]*) (?:command|file|tool|request)\b").expect("tool head"));

fn tool_of(note: &str, dialog: &DialogRead) -> Option<String> {
    if let Some(c) = TOOL_NOTE.captures(note) {
        return Some(c[1].to_string());
    }
    let first = dialog.body.as_deref()?.lines().next()?;
    TOOL_HEAD.captures(first).map(|c| c[1].to_string())
}

fn base_name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name).to_lowercase()
}

pub fn agent_of(info: &TermInfo) -> String {
    if let Some(agent) = info.labels.get("agent").map(|a| a.trim()).filter(|a| !a.is_empty()) {
        return agent.to_lowercase();
    }
    let Some(first) = info.cmd.first() else { return "unknown".into() };
    let program = base_name(first);
    if INTERPRETERS.contains(&program.as_str())
        && let Some(script) = info.cmd.iter().skip(1).find(|a| !a.starts_with('-'))
    {
        return base_name(script);
    }
    program
}

pub fn space_of(info: &TermInfo, spaces: &[Space]) -> Option<Space> {
    if let Some(id) = info.labels.get(crate::deep::SPACE_LABEL)
        && let Some(space) = crate::spaces::find(spaces, id)
    {
        return Some(space.clone());
    }
    crate::spaces::for_folder(spaces, &info.cwd).cloned()
}

pub fn screen_prompt(info: &TermInfo, dialog: &DialogRead, raw: &str, place: Option<String>, now: u64) -> Option<NewPrompt> {
    let note = info.status_note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let reading = if dialog.rows.is_empty() { read_screen(raw).filter(|r| r.state == ScreenState::Asking).map(|r| r.question).filter(|q| !q.is_empty()) } else { None };
    let stop_mark = note.as_deref() == Some(crate::kid_watch::MARK_NOTE);
    if dialog.rows.is_empty() && reading.is_none() && (note.is_none() || stop_mark) {
        return None;
    }
    let note_text = note.as_deref().unwrap_or("");
    let note_rules = dialog.rows.is_empty();
    let kind = if dialog.kind == DialogKind::Permission || (note_rules && PERMISSION_NOTE.is_match(note_text) && !PLAN_NOTE.is_match(note_text)) {
        PromptKind::Permission
    } else if dialog.kind == DialogKind::Plan || (note_rules && APPROVE_PLAN.is_match(note_text)) {
        PromptKind::Plan
    } else {
        PromptKind::Question
    };
    let agent = agent_of(info);
    let plain = kind == PromptKind::Question && reading.is_some() && agent != "claude";
    let unread = dialog.rows.is_empty() && !plain;
    let asked = match kind {
        PromptKind::Question => dialog.title.clone().or(reading.clone()).or(note.clone()),
        _ => note.clone().or(dialog.title.clone()).or(reading.clone()),
    };
    let tail = screen_tail(&dialog.raw, TAIL_LINES);
    let detail = dialog.body.clone().or_else(|| (dialog.rows.is_empty() && !tail.is_empty()).then_some(tail));
    let questions = if plain {
        vec![Question { question: asked.clone().unwrap_or_default(), options: plain_options(), ..Question::default() }]
    } else if kind == PromptKind::Question && !unread {
        vec![question_from_dialog(dialog)]
    } else {
        Vec::new()
    };
    let kind_word = match kind {
        PromptKind::Question => "question",
        PromptKind::Permission => "permission",
        PromptKind::Plan => "plan",
    };
    Some(NewPrompt {
        kind: kind_word.into(),
        terminal: info.id.to_string(),
        terminal_name: (!info.name.trim().is_empty()).then(|| info.name.clone()),
        agent,
        cwd: Some(info.cwd.clone()),
        place,
        unread,
        plain,
        text: asked,
        detail,
        tool_name: (kind == PromptKind::Permission).then(|| tool_of(note_text, dialog)).flatten(),
        screen_key: (!dialog.rows.is_empty()).then(|| dialog_key(dialog)),
        prompt_id: Some(format!("{SCREEN_PREFIX}{}:{now}", info.id)),
        questions,
        ..NewPrompt::default()
    })
}

fn add_carry(app: &App, note: &str) {
    let mut brain = lock(&app.brain);
    brain.carry = if brain.carry.is_empty() { note.to_string() } else { format!("{}\n\n{note}", brain.carry) };
}

pub fn told_text(app: &App, prompt: &Prompt, space: Option<&Space>) -> String {
    let head = format!("Event: {}", describe(prompt));
    if lock(&app.deep).owns(&prompt.terminal) {
        return format!("{head}{}", notes::deep_note());
    }
    let mut out = format!("{head}{}", notes::auto_note());
    let asked = prompt.question().map(|q| q.question.clone()).or_else(|| prompt.text.clone()).unwrap_or_default();
    let folder = prompt.cwd.clone().or_else(|| space.map(|s| s.folder.clone())).unwrap_or_default();
    if !folder.is_empty()
        && let Some(hit) = crate::answer_rules::for_question(app, &folder, &asked)
        && hit.act == crate::answer_rules::store::Act::Answer
    {
        out.push_str(&notes::rule_note(hit.rule.id, &hit.rule.answer));
    }
    let before = {
        let book = lock(&app.prompts.book);
        book.answered.iter().rev().find(|a| a.terminal == prompt.terminal && prompt.created_at.saturating_sub(a.at) < SAME_ASK_MS && same_ask(&a.asked, &asked)).cloned()
    };
    if let Some(before) = before {
        out.push_str(&notes::same_note(&before.id, &before.said));
    }
    out
}

pub fn untold_count(app: &App) -> usize {
    let book = lock(&app.prompts.book);
    book.items.iter().filter(|p| book.untold.contains(&p.id)).count()
}

pub fn tell_untold(app: &Arc<App>) -> usize {
    if !crate::settings_api::nixa_on(&app.dir) {
        return 0;
    }
    let waiting: Vec<Prompt> = {
        let mut book = lock(&app.prompts.book);
        let open: Vec<Prompt> = book.items.iter().filter(|p| book.untold.contains(&p.id)).cloned().collect();
        for p in &open {
            book.untold.remove(&p.id);
        }
        open
    };
    for prompt in &waiting {
        let space = app.space_of_terminal(&prompt.terminal);
        tell(app, prompt, space.as_ref());
    }
    if !waiting.is_empty() {
        log("prompts", &format!("told {} agent questions that waited", waiting.len()));
    }
    waiting.len()
}

pub fn tell(app: &Arc<App>, prompt: &Prompt, space: Option<&Space>) {
    if super::decide_first::take(app, prompt, space) {
        return;
    }
    tell_now(app, prompt, space, "");
}

pub const HARD_STOP_NOTE: &str = " Hard stop: only the user answers this prompt, on its card or in chat. Do not answer it with decide true; tell the user what waits.";

pub fn tell_now(app: &Arc<App>, prompt: &Prompt, space: Option<&Space>, extra: &str) {
    lock(&app.prompts.book).told_auto.insert(prompt.id.clone(), now_ms());
    let hard = super::decide_first::hard_stop(prompt);
    let routed = if hard { None } else { super::auto::route(app, prompt) };
    let stop = if hard { HARD_STOP_NOTE } else { "" };
    let text = match &routed {
        Some(ask) => format!("Event: {}{}{extra}", describe(prompt), notes::deep_routed_note(ask)),
        None => format!("{}{extra}{stop}", told_text(app, prompt, space)),
    };
    if !crate::agent::has_brain(app) {
        lock(&app.prompts.book).untold.insert(prompt.id.clone());
    }
    log("prompts", &format!("prompt {} from {} ({:?})", prompt.id, prompt.terminal, prompt.kind));
    crate::watch::tell_event(app, &text);
    if routed.is_none() && !prompt.unread {
        notify_parent(app, prompt, false);
    }
}

pub fn notify_parent(app: &Arc<App>, prompt: &Prompt, answered: bool) {
    let app = app.clone();
    let prompt = prompt.clone();
    tokio::spawn(async move {
        let link = app.prompts.link();
        let Ok(child) = link.get(&prompt.terminal).await else { return };
        let Some(parent) = child.parent.as_ref().map(ToString::to_string) else { return };
        let spaces = app.spaces();
        let Some(space) = space_of(&child, &spaces) else { return };
        if !lock(&app.deep).by_terminal(&parent).is_some_and(|seat| seat.space == space.id) {
            return;
        }
        let Ok(info) = link.get(&parent).await else { return };
        if !info.running || info.status == Some(Status::Blocked) {
            return;
        }
        let open = lock(&app.prompts.book).get(&prompt.id).is_some_and(|current| current.asked_at == prompt.asked_at);
        if !answered && !open {
            return;
        }
        let text = parent_notice(&prompt, answered);
        if let Err(error) = link.send(&parent, &text).await {
            log("prompts", &format!("could not notify parent {parent} about {}: {error}", prompt.id));
        }
    });
}

fn parent_notice(prompt: &Prompt, answered: bool) -> String {
    if answered {
        return format!("Nixa: The user answered prompt {} from your child terminal {}. Do not answer that step again.", prompt.id, prompt.terminal);
    }
    format!("Nixa: Your child terminal {} has an open prompt ({}). Decide it and answer it now with the prompt answer tool. Do not wait for the user.", prompt.terminal, prompt.id)
}

pub fn add_and_tell(app: &Arc<App>, event: NewPrompt, space: Option<&Space>) -> Option<Prompt> {
    let prompt = lock(&app.prompts.book).add(event, now_ms())?;
    tell(app, &prompt, space);
    broadcast(app);
    Some(prompt)
}

async fn check_blocked(app: &Arc<App>, link: &Link, info: &TermInfo, spaces: &[Space]) -> bool {
    let id = info.id.to_string();
    let (open, sending) = {
        let book = lock(&app.prompts.book);
        (book.by_terminal(&id), book.sending.clone())
    };
    if !open.is_empty() && (sending.is_some() || !open.iter().all(|p| p.unread || p.screen_key.is_some())) {
        return false;
    }
    let read = read_with_retry(link, &id).await;
    let from = read.from;
    let raw = read.text.unwrap_or_default();
    let dialog = parse_dialog(&raw);
    let mut changed = false;
    let now = now_ms();
    let space = space_of(info, spaces);
    let event = screen_prompt(info, &dialog, &raw, space.as_ref().map(|s| s.name.clone()), now);
    // A task status note can look like an unread prompt and block delivery of the task answer.
    if from == ReadFrom::Screen && dialog.kind == DialogKind::None
        && event.as_ref().is_some_and(|prompt| prompt.unread && prompt.kind == "question")
        && crate::queue::notes::terminal_waits_for_answer(app, &id) {
        let mut book = lock(&app.prompts.book);
        for p in open.iter().filter(|p| p.unread && p.kind == PromptKind::Question
            && p.prompt_id.as_deref().is_some_and(|id| id.starts_with(SCREEN_PREFIX))) {
            changed |= book.remove(&p.id);
        }
        return changed;
    }
    if !open.is_empty() {
        if dialog.rows.is_empty() {
            return from == ReadFrom::Screen && moved_on(&open, &dialog, &raw) && close_gone(app, &id, now);
        }
        lock(&app.prompts.book).missing.remove(&id);
        let key = dialog_key(&dialog);
        if open.iter().all(|p| p.screen_key.as_deref() == Some(key.as_str())) {
            return false;
        }
        let mut book = lock(&app.prompts.book);
        for p in &open {
            book.remove(&p.id);
        }
        changed = true;
        log("prompts", &format!("screen prompt changed in {id}"));
    }
    if dialog.rows.is_empty() && from == ReadFrom::Screen && WORKING.is_match(&raw) {
        return changed;
    }
    let Some(event) = event else { return changed };
    if lock(&app.prompts.book).sent_lately(&id, event.screen_key.as_deref(), now) {
        return changed;
    }
    let added = lock(&app.prompts.book).add(event, now_ms());
    if let Some(prompt) = added {
        tell(app, &prompt, space.as_ref());
        changed = true;
    }
    changed
}

pub fn moved_on(open: &[Prompt], dialog: &DialogRead, raw: &str) -> bool {
    if dialog.kind != DialogKind::None || raw.trim().is_empty() {
        return false;
    }
    let keyed = open.iter().all(|p| p.screen_key.is_some() && !p.unread);
    let working = open.iter().all(|p| p.unread) && WORKING.is_match(raw);
    keyed || working
}

fn close_gone(app: &App, id: &str, now: u64) -> bool {
    let mut book = lock(&app.prompts.book);
    let first = *book.missing.entry(id.to_string()).or_insert(now);
    if now.saturating_sub(first) < GONE_MS {
        return false;
    }
    book.missing.remove(id);
    let open = book.by_terminal(id);
    for p in &open {
        let told = !book.untold.contains(&p.id);
        book.remove(&p.id);
        log("prompts", &format!("prompt {} left the screen of {id}", p.id));
        if told {
            add_carry(app, &notes::on_screen_note(&p.id, id));
        }
    }
    !open.is_empty()
}

fn close_answered(app: &App, id: &str, now: u64) -> bool {
    let mut book = lock(&app.prompts.book);
    let open = book.by_terminal(id);
    let mut changed = false;
    for p in open {
        if book.sending.as_deref() == Some(p.id.as_str()) || now.saturating_sub(p.created_at) < HOOK_GRACE_MS {
            continue;
        }
        let told = !book.untold.contains(&p.id);
        book.remove(&p.id);
        changed = true;
        log("prompts", &format!("prompt {} answered on screen in {id}", p.id));
        if told {
            add_carry(app, &notes::on_screen_note(&p.id, id));
        }
    }
    changed
}

pub async fn scan(app: &Arc<App>, link: &Link) {
    let Ok(terms) = link.list().await else { return };
    let spaces = app.spaces();
    let now = now_ms();
    let mut changed = false;
    for info in terms.iter().filter(|t| t.running) {
        if info.status == Some(Status::Blocked) {
            changed |= check_blocked(app, link, info, &spaces).await;
        } else {
            changed |= close_answered(app, info.id.as_str(), now);
        }
    }
    {
        let live: Vec<String> = terms.iter().filter(|t| t.running).map(|t| t.id.to_string()).collect();
        let mut book = lock(&app.prompts.book);
        let gone: Vec<String> = book.items.iter().filter(|p| !live.contains(&p.terminal)).map(|p| p.id.clone()).collect();
        for id in gone {
            book.remove(&id);
            changed = true;
        }
    }
    if changed {
        broadcast(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::dialog::parse_dialog;
    use nixa_protocol::TermId;

    #[test]
    fn parent_notice_distinguishes_open_and_clicked_steps() {
        let mut book = super::super::book::Book::default();
        let prompt = book.add(NewPrompt { kind: "question".into(), terminal: "m7".into(), ..NewPrompt::default() }, 1).unwrap();
        let decided = parent_notice(&prompt, false);
        assert!(decided.contains("m7") && decided.contains(&prompt.id) && decided.contains("open prompt"));
        assert!(decided.contains("answer it now") && !decided.contains("authorized"));
        let clicked = parent_notice(&prompt, true);
        assert!(clicked.contains("m7") && clicked.contains(&prompt.id) && clicked.contains("Do not answer that step again"));
    }

    pub fn info(cmd: &[&str], note: Option<&str>) -> TermInfo {
        TermInfo {
            id: TermId::from_number(7),
            name: "agent".into(),
            labels: Default::default(),
            parent: None,
            children: vec![],
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            cwd: "/tmp/shop".into(),
            pid: None,
            running: true,
            exit_code: None,
            status: Some(Status::Blocked),
            status_note: note.map(str::to_string),
            keep: false,
            draft: false,
            title: None,
            rows: 24,
            cols: 80,
            created_ms: 0,
            seq: 1,
        }
    }

    #[test]
    fn a_live_answer_rule_is_offered_to_the_brain() {
        use crate::answer_rules::store::{self, RuleInput, RuleStatus};
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let prompt = Prompt {
            id: "p1".into(), kind: PromptKind::Permission, agent: "codex".into(), terminal: "m4".into(), terminal_name: None, cwd: Some("/tmp/shop".into()), place: None,
            unread: false, plain: false, text: Some("Do you want me to run the tests?".into()), tool_name: None, detail: None,
            screen_key: None, prompt_id: None, questions: Vec::new(), question_index: 0, created_at: 0, asked_at: 0,
        };
        let mut rule = RuleInput { id: None, project: "*".into(), ask: "run the tests".into(), answer: "yes, run them".into(), status: RuleStatus::Trial, expected: None };
        store::save(&app.dir, &rule).unwrap();
        assert!(!told_text(&app, &prompt, None).contains("answer rule"));
        rule.id = Some(store::load(&app.dir).unwrap()[0].id);
        rule.status = RuleStatus::Live;
        rule.expected = Some(store::stamp(&store::load(&app.dir).unwrap()[0]));
        store::save(&app.dir, &rule).unwrap();
        let told = told_text(&app, &prompt, None);
        assert!(told.contains("live answer rule") && told.contains("yes, run them"), "{told}");
    }

    #[test]
    fn a_stopped_child_mark_is_not_a_question() {
        let quiet = "⏺ Done with the first part.\n\n❯ \n";
        let marked = info(&["claude"], Some(crate::kid_watch::MARK_NOTE));
        assert!(screen_prompt(&marked, &parse_dialog(quiet), quiet, None, 5).is_none());
        let asking = "Which database should I use?\n\n❯ 1. SQLite\n  2. Postgres\n\nEnter to select · Esc to cancel\n";
        assert!(screen_prompt(&marked, &parse_dialog(asking), asking, None, 5).is_some());
        let other = info(&["claude"], Some("Which branch should I use?"));
        assert!(screen_prompt(&other, &parse_dialog(quiet), quiet, None, 5).is_some_and(|p| p.unread));
    }

    #[test]
    fn a_card_closes_when_its_dialog_leaves_the_screen() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let screen = "Which database should I use?\n\n❯ 1. SQLite\n  2. Postgres\n\nEnter to select · Esc to cancel\n";
        let event = screen_prompt(&info(&["claude"], None), &parse_dialog(screen), screen, None, 5).unwrap();
        let p = lock(&app.prompts.book).add(event, 1).unwrap();
        let open = vec![p.clone()];
        let resumed = "⏺ Using Postgres.\n✻ Working… (esc to interrupt)\n";
        assert!(moved_on(&open, &parse_dialog(resumed), resumed));
        assert!(!moved_on(&open, &parse_dialog(screen), screen));
        assert!(!moved_on(&open, &parse_dialog(""), ""));
        let mut unread = p.clone();
        unread.unread = true;
        unread.screen_key = None;
        assert!(moved_on(std::slice::from_ref(&unread), &parse_dialog(resumed), resumed));
        assert!(!moved_on(&[unread], &parse_dialog("waiting for input"), "waiting for input"));
        assert!(!close_gone(&app, &p.terminal, 10_000));
        assert!(lock(&app.prompts.book).get(&p.id).is_some());
        assert!(close_gone(&app, &p.terminal, 10_000 + GONE_MS));
        assert!(lock(&app.prompts.book).items.is_empty());
        assert!(lock(&app.prompts.book).missing.is_empty());
        assert!(lock(&app.brain).carry.contains(&format!("answered prompt {}", p.id)));
        assert!(super::super::snapshot(&app)["card"].is_null());
    }

    #[test]
    fn agent_names() {
        assert_eq!(agent_of(&info(&["/usr/local/bin/claude", "--resume"], None)), "claude");
        assert_eq!(agent_of(&info(&["python3", "-u", "/x/fake_agent.py"], None)), "fake_agent");
        assert_eq!(agent_of(&info(&[], None)), "unknown");
        let mut labeled = info(&["node", "x.js"], None);
        labeled.labels.insert("agent".into(), "Codex".into());
        assert_eq!(agent_of(&labeled), "codex");
    }

    #[test]
    fn menus_become_questions() {
        let screen = "Which database should I use?\n\n❯ 1. SQLite\n  2. Postgres\n\nEnter to select · Esc to cancel\n";
        let t = info(&["claude"], None);
        let e = screen_prompt(&t, &parse_dialog(screen), screen, Some("shop".into()), 5).unwrap();
        assert_eq!(e.kind, "question");
        assert!(!e.unread && !e.plain);
        assert_eq!(e.text.as_deref(), Some("Which database should I use?"));
        assert_eq!(e.questions[0].options.len(), 2);
        assert_eq!(e.prompt_id.as_deref(), Some("screen:m7:5"));
        assert!(e.screen_key.is_some());
    }

    #[test]
    fn permission_by_screen_or_note() {
        let screen = "Bash command\n  ls\nDo you want to proceed?\n❯ 1. Yes\n  2. No\n";
        let e = screen_prompt(&info(&["claude"], None), &parse_dialog(screen), screen, None, 1).unwrap();
        assert_eq!(e.kind, "permission");
        assert_eq!(e.tool_name.as_deref(), Some("Bash"));
        assert!(e.questions.is_empty());
        let e = screen_prompt(&info(&["claude"], Some("Claude needs your permission to use Bash")), &parse_dialog("waiting"), "waiting", None, 1).unwrap();
        assert_eq!(e.kind, "permission");
        assert_eq!(e.tool_name.as_deref(), Some("Bash"));
        assert!(e.unread);
        let e = screen_prompt(&info(&["claude"], Some("approve plan")), &parse_dialog("x"), "x", None, 1).unwrap();
        assert_eq!(e.kind, "plan");
    }

    #[test]
    fn a_note_does_not_hide_the_menu_on_screen() {
        let screen = "Which database should I use?\n\n❯ 1. SQLite\n  2. Postgres\n\nEnter to select · Esc to cancel\n";
        let t = info(&["claude"], Some("Claude needs your permission to use the editor"));
        let e = screen_prompt(&t, &parse_dialog(screen), screen, None, 1).unwrap();
        assert_eq!(e.kind, "question");
        assert_eq!(e.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), vec!["SQLite", "Postgres"]);
        let t = info(&["claude"], Some("approve plan"));
        assert_eq!(screen_prompt(&t, &parse_dialog(screen), screen, None, 1).unwrap().kind, "question");
    }

    #[test]
    fn plain_text_questions() {
        let mut blocked = info(&[], Some("waiting for input"));
        blocked.name = "build".into();
        let prompt = screen_prompt(&blocked, &parse_dialog(""), "", None, 1).unwrap();
        assert!(prompt.unread && !prompt.plain && prompt.questions.is_empty());
        assert_eq!(prompt.terminal_name.as_deref(), Some("build"));
        let screen = "I fixed the parser.\nShould I run the full test suite now?\n";
        let e = screen_prompt(&info(&["codex"], None), &parse_dialog(screen), screen, None, 1).unwrap();
        assert!(e.plain && !e.unread);
        assert_eq!(e.text.as_deref(), Some("Should I run the full test suite now?"));
        assert_eq!(e.questions[0].options.len(), 2);
        let claude = screen_prompt(&info(&["claude"], None), &parse_dialog(screen), screen, None, 1).unwrap();
        assert!(claude.unread && !claude.plain);
        assert!(screen_prompt(&info(&["codex"], None), &parse_dialog("All done."), "All done.", None, 1).is_none());
    }

    #[test]
    fn printed_steps_above_the_input_box_are_not_a_prompt() {
        let steps = "⏺ Staged and checked. To activate it:\n  1. Copy the app into place\n  2. Restart the brain\n\n────────────────────────\n❯ \n────────────────────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle)\n";
        assert!(screen_prompt(&info(&["claude"], None), &parse_dialog(steps), steps, None, 1).is_none());
        let asked = "Which database should I use?\n1. SQLite\n2. Postgres\n\n› \n";
        let e = screen_prompt(&info(&["codex"], None), &parse_dialog(asked), asked, None, 1).unwrap();
        assert!(e.plain && e.screen_key.is_none());
        assert_eq!(e.text.as_deref(), Some("Which database should I use?"));
    }
}
