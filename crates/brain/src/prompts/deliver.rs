use super::dialog::{DialogRead, Question, REVIEW, YES_NO, empty_dialog, find_row, highlighted, parse_dialog, row_for};
use super::link::Link;
use nixa_protocol::{Key, Status};
use regex::Regex;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    pub ok: bool,
    pub detail: String,
    pub next: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allow,
    Always,
    Deny,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("deliver pattern")
}

static SUBMIT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^submit"));
static TYPE_ROW: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(type something|other)"));
static ALLOW: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(yes|allow|approve|accept|proceed|y)\b"));
static ALLOW_SKIP: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)auto mode|bypass|and "));
static ALWAYS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^yes, and (allow|don't ask|always)|always|don't ask again"));
static ALWAYS_SKIP: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)auto mode|bypass"));
static ALLOW_SHORT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(yes|allow|approve)\b"));
static DENY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(no|deny|reject|cancel|n)\b"));
static MANUAL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)manually approve"));
static AUTO_ACCEPT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)auto[- ]?accept"));
static YES: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^yes"));
static BYPASS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)bypass"));
static FEEDBACK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)what to change|feedback|^no\b"));

pub const POLL_MS: u64 = 300;
pub const MOVE_MS: u64 = 350;
pub const ENTER_MS: u64 = 700;
pub const SETTLE_MS: u64 = 500;
pub const DIALOG_WAIT_MS: u64 = 4000;
pub const GONE_WAIT_MS: u64 = 3000;

fn done(ok: bool, detail: impl Into<String>) -> Delivered {
    Delivered { ok, detail: detail.into(), next: false }
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

pub async fn read_dialog(link: &Link, terminal: &str) -> DialogRead {
    match link.read(terminal).await {
        Ok(text) => parse_dialog(&text),
        Err(_) => empty_dialog(""),
    }
}

pub async fn wait_for_dialog(link: &Link, terminal: &str, timeout_ms: u64) -> DialogRead {
    let until = Instant::now() + Duration::from_millis(timeout_ms);
    let mut last = empty_dialog("");
    while Instant::now() < until {
        last = read_dialog(link, terminal).await;
        if !last.rows.is_empty() {
            return last;
        }
        pause(POLL_MS).await;
    }
    last
}

pub async fn wait_for_dialog_gone(link: &Link, terminal: &str, timeout_ms: u64) -> bool {
    let until = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < until {
        if read_dialog(link, terminal).await.rows.is_empty() {
            return true;
        }
        pause(POLL_MS).await;
    }
    false
}

async fn press(link: &Link, terminal: &str, keys: Vec<Key>) -> bool {
    link.keys(terminal, keys).await.is_ok()
}

async fn move_to(link: &Link, terminal: &str, pos: usize) -> Option<DialogRead> {
    for _ in 0..3 {
        let d = read_dialog(link, terminal).await;
        if d.rows.is_empty() || pos >= d.rows.len() {
            return None;
        }
        let cur = highlighted(&d);
        if cur == Some(pos) {
            return Some(d);
        }
        let from = cur.unwrap_or(0) as i64;
        let delta = pos as i64 - from;
        let key = if delta > 0 { Key::Down } else { Key::Up };
        let count = if cur.is_none() { pos } else { delta.unsigned_abs() as usize };
        if count == 0 {
            return Some(d);
        }
        if !press(link, terminal, vec![key; count]).await {
            return None;
        }
        pause(MOVE_MS).await;
    }
    let d = read_dialog(link, terminal).await;
    (highlighted(&d) == Some(pos)).then_some(d)
}

async fn press_enter(link: &Link, terminal: &str) -> bool {
    let ok = press(link, terminal, vec![Key::Enter]).await;
    pause(ENTER_MS).await;
    ok
}

async fn select_row(link: &Link, terminal: &str, pos: usize) -> Delivered {
    let Some(d) = move_to(link, terminal, pos).await else { return done(false, format!("could not reach option {}", pos + 1)) };
    let label = d.rows[pos].label.clone();
    if !press_enter(link, terminal).await {
        return done(false, "the key press failed");
    }
    crate::util::log("prompts", &format!("selected row {} in {terminal}: {label}", pos + 1));
    done(true, label)
}

async fn after_question_step(link: &Link, terminal: &str) -> Delivered {
    let d = read_dialog(link, terminal).await;
    if d.rows.is_empty() {
        return done(true, "done");
    }
    if REVIEW.is_match(&d.raw) {
        let pos = find_row(&d, &SUBMIT, None).unwrap_or(0);
        let r = select_row(link, terminal, pos).await;
        if !r.ok {
            return r;
        }
        wait_for_dialog_gone(link, terminal, GONE_WAIT_MS).await;
        return done(true, "submitted");
    }
    Delivered { ok: true, detail: "next question".into(), next: true }
}

fn option_pos(d: &DialogRead, index: usize, question: Option<&Question>) -> Option<usize> {
    if let Some(wanted) = question.and_then(|q| q.options.get(index)).map(|o| o.label.as_str())
        && let Some(pos) = row_for(&d.rows, wanted)
    {
        return Some(pos);
    }
    (index < d.rows.len()).then_some(index)
}

pub async fn answer_option(link: &Link, terminal: &str, index: usize, question: Option<&Question>) -> Delivered {
    let d = wait_for_dialog(link, terminal, DIALOG_WAIT_MS).await;
    if d.rows.is_empty() {
        return done(false, "no dialog on screen");
    }
    let Some(pos) = option_pos(&d, index, question) else { return done(false, format!("option {} is not on screen", index + 1)) };
    let r = select_row(link, terminal, pos).await;
    if !r.ok {
        return r;
    }
    let after = after_question_step(link, terminal).await;
    Delivered { detail: r.detail, ..after }
}

pub async fn answer_options(link: &Link, terminal: &str, indexes: &[usize], question: Option<&Question>) -> Delivered {
    let d = wait_for_dialog(link, terminal, DIALOG_WAIT_MS).await;
    if d.rows.is_empty() {
        return done(false, "no dialog on screen");
    }
    let mut labels = Vec::new();
    for &index in indexes {
        let Some(pos) = option_pos(&d, index, question) else { return done(false, format!("option {} is not on screen", index + 1)) };
        let Some(moved) = move_to(link, terminal, pos).await else { return done(false, format!("could not reach option {}", index + 1)) };
        if moved.rows[pos].checked != Some(true) && !press_enter(link, terminal).await {
            return done(false, "the key press failed");
        }
        labels.push(moved.rows[pos].label.clone());
    }
    if !press(link, terminal, vec![Key::Tab]).await {
        return done(false, "the key press failed");
    }
    pause(SETTLE_MS).await;
    let after = after_question_step(link, terminal).await;
    Delivered { detail: labels.join(", "), ..after }
}

pub async fn type_text(link: &Link, terminal: &str, text: &str) -> Delivered {
    match link.send(terminal, text).await {
        Ok(()) => done(true, format!("typed: {text}")),
        Err(e) => done(false, format!("send failed: {e}")),
    }
}

pub async fn answer_text(link: &Link, terminal: &str, text: &str) -> Delivered {
    let d = wait_for_dialog(link, terminal, DIALOG_WAIT_MS).await;
    match find_row(&d, &TYPE_ROW, None) {
        Some(pos) => {
            let r = select_row(link, terminal, pos).await;
            if !r.ok {
                return r;
            }
        }
        None => {
            press(link, terminal, vec![Key::Esc]).await;
            pause(SETTLE_MS).await;
        }
    }
    wait_for_dialog_gone(link, terminal, 2000).await;
    type_text(link, terminal, text).await
}

pub fn permission_row(d: &DialogRead, choice: Permission) -> Option<usize> {
    match choice {
        Permission::Allow => find_row(d, &ALLOW, Some(&ALLOW_SKIP)),
        Permission::Always => find_row(d, &ALWAYS, Some(&ALWAYS_SKIP)).or_else(|| find_row(d, &ALLOW_SHORT, Some(&ALLOW_SKIP))),
        Permission::Deny => find_row(d, &DENY, None),
    }
}

pub async fn answer_permission(link: &Link, terminal: &str, choice: Permission) -> Delivered {
    let d = wait_for_dialog(link, terminal, DIALOG_WAIT_MS).await;
    if d.rows.is_empty() {
        if YES_NO.is_match(&d.raw) {
            let key = if choice == Permission::Deny { "n" } else { "y" };
            if !press(link, terminal, vec![Key::Raw(key.into()), Key::Enter]).await {
                return done(false, "the key press failed");
            }
            return done(true, format!("{choice:?}").to_lowercase());
        }
        return done(false, "no dialog on screen");
    }
    let Some(pos) = permission_row(&d, choice) else {
        if choice == Permission::Deny {
            press(link, terminal, vec![Key::Esc]).await;
            pause(SETTLE_MS).await;
            return done(true, "cancelled with escape");
        }
        return done(false, format!("no row on screen fits {}", format!("{choice:?}").to_lowercase()));
    };
    let r = select_row(link, terminal, pos).await;
    if r.ok {
        wait_for_dialog_gone(link, terminal, GONE_WAIT_MS).await;
    }
    r
}

pub fn plan_row(d: &DialogRead, auto: bool) -> Option<usize> {
    let auto_row = if auto { find_row(d, &AUTO_ACCEPT, Some(&BYPASS)) } else { None };
    auto_row.or_else(|| find_row(d, &MANUAL, None)).or_else(|| find_row(d, &YES, Some(&BYPASS))).or_else(|| find_row(d, &YES, None))
}

pub async fn answer_plan(link: &Link, terminal: &str, approve: bool, text: Option<&str>, auto: bool) -> Delivered {
    let d = wait_for_dialog(link, terminal, DIALOG_WAIT_MS).await;
    if d.rows.is_empty() {
        return done(false, "no dialog on screen");
    }
    if approve {
        let Some(pos) = plan_row(&d, auto) else { return done(false, "no approve row") };
        let r = select_row(link, terminal, pos).await;
        if r.ok {
            wait_for_dialog_gone(link, terminal, GONE_WAIT_MS).await;
        }
        return r;
    }
    if let (Some(pos), Some(text)) = (find_row(&d, &FEEDBACK, None), text) {
        let r = select_row(link, terminal, pos).await;
        if !r.ok {
            return r;
        }
        wait_for_dialog_gone(link, terminal, 2000).await;
        let sent = type_text(link, terminal, text).await;
        return if sent.ok { done(true, format!("rejected with: {text}")) } else { sent };
    }
    press(link, terminal, vec![Key::Esc]).await;
    pause(SETTLE_MS).await;
    if let Some(text) = text {
        let sent = type_text(link, terminal, text).await;
        if !sent.ok {
            return sent;
        }
        return done(true, format!("rejected with: {text}"));
    }
    done(true, "rejected")
}

pub async fn still_waiting(link: &Link, terminal: &str) -> bool {
    match link.get(terminal).await {
        Ok(term) if !term.running => false,
        Ok(term) if term.status == Some(Status::Blocked) => true,
        Ok(term) if term.status.is_none() => false,
        Ok(_) => !read_dialog(link, terminal).await.rows.is_empty(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_never_falls_back_to_the_first_row() {
        let odd = parse_dialog("Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. No\n  2. Sure, go on\n");
        assert_eq!(odd.rows.len(), 2);
        assert_eq!(permission_row(&odd, Permission::Allow), None);
        assert_eq!(permission_row(&odd, Permission::Deny), Some(0));
        let always_first = parse_dialog("Bash command\n  ls\nDo you want to proceed?\n❯ 1. Yes, and don't ask again for ls\n  2. Yes\n  3. No\n");
        assert_eq!(permission_row(&always_first, Permission::Allow), Some(1));
        assert_eq!(permission_row(&always_first, Permission::Always), Some(0));
        assert_eq!(permission_row(&always_first, Permission::Deny), Some(2));
    }

    #[test]
    fn plan_approval_auto_accepts_in_automatic() {
        let plan = parse_dialog("Claude has written up a plan

Would you like to proceed?
❯ 1. Yes, and auto-accept edits
  2. Yes, and manually approve edits
  3. No, keep planning
");
        assert_eq!(plan_row(&plan, true), Some(0));
        assert_eq!(plan_row(&plan, false), Some(1));
    }
}
