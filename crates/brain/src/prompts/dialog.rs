use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogKind {
    None,
    Unknown,
    Permission,
    Plan,
    Trust,
    Question,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogRow {
    pub index: usize,
    pub label: String,
    pub highlighted: bool,
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogRead {
    pub kind: DialogKind,
    pub title: Option<String>,
    pub rows: Vec<DialogRow>,
    pub footer: Option<String>,
    pub body: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("dialog pattern")
}

static ROW: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*([❯›>•])?\s*(?:\[([ ✔✓x✗])\]\s*)?(\d{1,2})\.\s+(?:\[([ ✔✓x✗])\]\s*)?(.+?)\s*$"));
static CHECK_ROW: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*([❯›>•])?\s*\[([ ✔✓x✗])\]\s*(.+?)\s*$"));
static PROMPT_ECHO: LazyLock<Regex> = LazyLock::new(|| re(r"<nixa from="));
static SEPARATOR: LazyLock<Regex> = LazyLock::new(|| re(r"^[\s─╌━═-]{6,}$"));
static INPUT_BOX: LazyLock<Regex> = LazyLock::new(|| re(r"^[\s│┃]*[❯›>](?:\s.*)?$"));
static FOOTER: LazyLock<Regex> = LazyLock::new(|| re(r"(Enter to select|Esc to cancel|Tab to amend|↑/↓ to navigate|ctrl\+g to edit)"));
static TABS: LazyLock<Regex> = LazyLock::new(|| re(r"(☐|☒)\s"));
static PERMISSION: LazyLock<Regex> = LazyLock::new(|| re(r"Do you want to proceed\?"));
static PLAN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(written up a plan|Would you like to proceed\?|approve plan)"));
static TRUST: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)Do you trust the files"));
pub static REVIEW: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)Review your answers"));
pub static YES_NO: LazyLock<Regex> = LazyLock::new(|| re(r"\[(y/n|Y/n|y/N)\]|\((y/n|Y/n|y/N)\)"));
static TIP: LazyLock<Regex> = LazyLock::new(|| re(r"^Tip:"));
static TYPE_SOMETHING: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^type something"));
static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[^a-z0-9]+"));
static KEY_NOISE: LazyLock<Regex> = LazyLock::new(|| re(r"[0-9⠀-⣿✻✶✳✢✽·∙●◐◓◑◒◴◵◶◷○◌◎]"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));

pub fn empty_dialog(raw: &str) -> DialogRead {
    DialogRead { kind: DialogKind::None, title: None, rows: Vec::new(), footer: None, body: None, raw: raw.to_string() }
}

fn checked(mark: Option<regex::Match<'_>>) -> Option<bool> {
    mark.map(|m| m.as_str() != " ")
}

fn quiet_line(line: &str) -> bool {
    line.is_empty() || SEPARATOR.is_match(line) || PROMPT_ECHO.is_match(line) || TIP.is_match(line)
}

fn title_above(lines: &[&str], first_row: usize) -> Option<String> {
    let low = first_row.saturating_sub(8);
    (low..first_row).rev().map(|i| lines[i].trim()).find(|line| !quiet_line(line) && !TABS.is_match(line)).map(str::to_string)
}

struct Found {
    line: usize,
    number: Option<usize>,
    row: DialogRow,
}

pub fn parse_dialog(screen: &str) -> DialogRead {
    let text = screen.replace('\r', "");
    let all: Vec<&str> = text.split('\n').collect();
    let lines = &all[all.len().saturating_sub(80)..];
    let mut found: Vec<Found> = Vec::new();
    let mut footer = None;
    let mut input_at = None;
    for (i, line) in lines.iter().enumerate() {
        if PROMPT_ECHO.is_match(line) {
            continue;
        }
        if let Some(m) = ROW.captures(line) {
            let number: usize = m[3].parse().unwrap_or(0);
            let row = DialogRow { index: number.saturating_sub(1), label: m[5].to_string(), highlighted: m.get(1).is_some(), checked: checked(m.get(2).or(m.get(4))) };
            found.push(Found { line: i, number: Some(number.saturating_sub(1)), row });
            continue;
        }
        if let Some(c) = CHECK_ROW.captures(line) {
            let row = DialogRow { index: 0, label: c[3].to_string(), highlighted: c.get(1).is_some(), checked: checked(c.get(2)) };
            found.push(Found { line: i, number: None, row });
            continue;
        }
        if FOOTER.is_match(line) {
            footer = Some(line.trim().to_string());
        }
        if INPUT_BOX.is_match(line) {
            input_at = Some(i);
        }
    }
    let raw = lines.join("\n");
    let start = found.iter().rposition(|f| f.number == Some(0)).or_else(|| found.first().filter(|f| f.number.is_none()).map(|_| 0));
    let mut rows: Vec<DialogRow> = Vec::new();
    if let Some(start) = start {
        let mut expect = found[start].number;
        for f in &found[start..] {
            match f.number {
                None => rows.push(DialogRow { index: rows.len(), ..f.row.clone() }),
                Some(n) if Some(n) == expect => {
                    rows.push(f.row.clone());
                    expect = Some(n + 1);
                }
                Some(_) => break,
            }
        }
    }
    let printed = !rows.iter().any(|r| r.highlighted) && start.zip(input_at).is_some_and(|(s, at)| at > found[s].line);
    let Some(start) = start.filter(|_| !rows.is_empty() && !printed) else {
        let kind = if YES_NO.is_match(&raw) { DialogKind::Unknown } else { DialogKind::None };
        return DialogRead { kind, footer, ..empty_dialog(&raw) };
    };
    let first_line = found[start].line;
    let title = title_above(lines, first_line);
    let from = first_line.saturating_sub(12);
    let tail = lines[from..].join("\n");
    let body: Vec<&str> = lines[from..first_line].iter().map(|l| l.trim()).filter(|l| !quiet_line(l)).collect();
    let kind = if PERMISSION.is_match(&tail) {
        DialogKind::Permission
    } else if PLAN.is_match(&tail) {
        DialogKind::Plan
    } else if TRUST.is_match(&tail) {
        DialogKind::Trust
    } else if REVIEW.is_match(&tail) || TABS.is_match(&tail) || footer.as_deref().is_some_and(|f| FOOTER.is_match(f)) {
        DialogKind::Question
    } else {
        DialogKind::Unknown
    };
    let body = (!body.is_empty()).then(|| body.join("\n"));
    DialogRead { kind, title, rows, footer, body, raw }
}

pub fn question_from_dialog(dialog: &DialogRead) -> Question {
    Question {
        question: dialog.title.clone().unwrap_or_else(|| "Which option?".into()),
        header: None,
        options: dialog.rows.iter().filter(|r| !TYPE_SOMETHING.is_match(&r.label)).map(|r| QuestionOption { label: r.label.clone(), description: None }).collect(),
        multi_select: dialog.rows.iter().any(|r| r.checked.is_some()),
    }
}

fn plain_words(text: &str) -> String {
    NOT_WORD.replace_all(&text.to_lowercase(), " ").trim().to_string()
}

pub fn row_for(rows: &[DialogRow], wanted: &str) -> Option<usize> {
    let want = plain_words(wanted);
    let labels: Vec<String> = rows.iter().map(|r| plain_words(&r.label)).collect();
    if let Some(exact) = labels.iter().position(|l| *l == want) {
        return Some(exact);
    }
    let cut = labels.iter().enumerate().filter(|(_, l)| !l.is_empty() && want.starts_with(l.as_str())).max_by_key(|(i, l)| (l.len(), std::cmp::Reverse(*i))).map(|(i, _)| i);
    cut.or_else(|| {
        labels.iter().enumerate().filter(|(_, l)| !want.is_empty() && l.starts_with(want.as_str())).min_by_key(|(i, l)| (l.len(), *i)).map(|(i, _)| i)
    })
}

pub fn dialog_key(dialog: &DialogRead) -> String {
    let labels: Vec<&str> = dialog.rows.iter().map(|r| r.label.as_str()).collect();
    let head = dialog.body.as_deref().or(dialog.title.as_deref()).unwrap_or("");
    let text = format!("{head}|{}", labels.join("|"));
    SPACES.replace_all(&KEY_NOISE.replace_all(&text, ""), " ").trim().to_string()
}

pub fn screen_tail(raw: &str, count: usize) -> String {
    let lines: Vec<&str> = raw.split('\n').map(str::trim).filter(|l| !l.is_empty() && !SEPARATOR.is_match(l) && !PROMPT_ECHO.is_match(l)).collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

pub fn find_row(dialog: &DialogRead, test: &Regex, exclude: Option<&Regex>) -> Option<usize> {
    dialog.rows.iter().position(|r| test.is_match(&r.label) && !exclude.is_some_and(|x| x.is_match(&r.label)))
}

pub fn highlighted(dialog: &DialogRead) -> Option<usize> {
    dialog.rows.iter().position(|r| r.highlighted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUESTION: &str = "Some work above\n\n ☐ Database\n\nWhich database should I use?\n\n❯ 1. SQLite\n     Small and local\n  2. Postgres\n  3. Type something.\n\nEnter to select · ↑/↓ to navigate · Esc to cancel\n";
    const PERMISSION_SCREEN: &str = "Bash command\n\n  rm -rf build\n  Remove the build folder\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again for rm commands\n  3. No, and tell Claude what to do differently (esc)\n";

    #[test]
    fn reads_a_question_dialog() {
        let d = parse_dialog(QUESTION);
        assert_eq!(d.kind, DialogKind::Question);
        assert_eq!(d.title.as_deref(), Some("Which database should I use?"));
        assert_eq!(d.rows.len(), 3);
        assert!(d.rows[0].highlighted);
        assert!(!d.rows[1].highlighted);
        assert_eq!(d.rows[1].label, "Postgres");
        let q = question_from_dialog(&d);
        assert_eq!(q.options.len(), 2);
        assert!(!q.multi_select);
        assert!(d.footer.is_some());
    }

    #[test]
    fn reads_a_permission_dialog() {
        let d = parse_dialog(PERMISSION_SCREEN);
        assert_eq!(d.kind, DialogKind::Permission);
        assert_eq!(d.rows.len(), 3);
        assert_eq!(d.title.as_deref(), Some("Do you want to proceed?"));
        assert!(d.body.as_deref().unwrap().contains("rm -rf build"));
    }

    #[test]
    fn reads_plan_and_trust() {
        let plan = parse_dialog("Claude has written up a plan\n\nWould you like to proceed?\n❯ 1. Yes, and auto-accept edits\n  2. Yes, and manually approve edits\n  3. No, keep planning\n");
        assert_eq!(plan.kind, DialogKind::Plan);
        let trust = parse_dialog("Do you trust the files in this folder?\n❯ 1. Yes, proceed\n  2. No, exit\n");
        assert_eq!(trust.kind, DialogKind::Trust);
    }

    #[test]
    fn reads_check_rows() {
        let d = parse_dialog("Pick the parts\n❯ 1. [ ] Tests\n  2. [✔] Docs\n  3. [ ] Lint\n\nEnter to select · Tab to amend\n");
        assert_eq!(d.rows.len(), 3);
        assert_eq!(d.rows[1].checked, Some(true));
        assert_eq!(d.rows[0].checked, Some(false));
        assert!(question_from_dialog(&d).multi_select);
        let bare = parse_dialog("Choose\n› [ ] alpha\n  [x] beta\n");
        assert_eq!(bare.rows.len(), 2);
        assert_eq!(bare.rows[1].index, 1);
    }

    #[test]
    fn last_list_wins_and_gaps_stop() {
        let d = parse_dialog("1. old\n2. older\n\nPick one?\n❯ 1. new\n  2. newer\n  4. skipped\n");
        assert_eq!(d.rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), vec!["new", "newer"]);
    }

    #[test]
    fn no_rows_and_yes_no() {
        assert_eq!(parse_dialog("just text").kind, DialogKind::None);
        assert_eq!(parse_dialog("Overwrite? [y/N]").kind, DialogKind::Unknown);
        assert!(parse_dialog("<nixa from=\"m1\">\n1. a\n").rows.len() == 1);
    }

    #[test]
    fn numbered_output_above_the_input_box_is_not_a_dialog() {
        let screen = "⏺ To activate the staged build:\n  1. Copy the app into place\n  2. Restart the brain\n\n────────────────────────\n❯ \n────────────────────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle)\n";
        let d = parse_dialog(screen);
        assert!(d.rows.is_empty());
        assert_eq!(d.kind, DialogKind::None);
        let boxed = parse_dialog("Steps:\n1. Build\n2. Install\n╭──────────╮\n│ >        │\n╰──────────╯\n");
        assert!(boxed.rows.is_empty());
        let live = parse_dialog("Steps:\n1. Build\n2. Install\n\n> go on\n\nDo you want to proceed?\n❯ 1. Yes\n  2. No\n");
        assert_eq!(live.kind, DialogKind::Permission);
        assert_eq!(live.rows.len(), 2);
        assert_eq!(parse_dialog(QUESTION).rows.len(), 3);
    }

    #[test]
    fn finds_rows_by_label() {
        let d = parse_dialog(QUESTION);
        assert_eq!(row_for(&d.rows, "Postgres"), Some(1));
        assert_eq!(row_for(&d.rows, "postgres!"), Some(1));
        assert_eq!(row_for(&d.rows, "SQLite (recommended)"), Some(0));
        assert_eq!(row_for(&d.rows, "Type"), Some(2));
        assert_eq!(row_for(&d.rows, "MySQL"), None);
    }

    #[test]
    fn keys_ignore_spinners_and_numbers() {
        let a = parse_dialog(&QUESTION.replace("Some work above", "✻ Thinking 12s"));
        let b = parse_dialog(&QUESTION.replace("Some work above", "· Thinking 47s"));
        assert_eq!(dialog_key(&a), dialog_key(&b));
        assert!(!dialog_key(&a).is_empty());
    }

    #[test]
    fn tail_skips_lines() {
        assert_eq!(screen_tail("a\n\n------\nb\nc\n", 2), "b\nc");
    }
}
