use crate::chat::{ChatItem, RECAP_CHARS, RECAP_LINES, Who, in_brain_context, line_of};
use crate::util::{atomic_write, now_ms, short_hash};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const COMPACT_AT: f64 = 0.75;
pub const FULL_AT: f64 = 0.9;
pub const MAX_BUDGET: u64 = 120_000;
pub const MIN_GAIN: f64 = 0.2;
pub const MIN_GAP_MS: u64 = 5 * 60 * 1000;
pub const SUMMARY_CHARS: usize = 3000;
pub const LINE_CHARS: usize = 220;
pub const KEEP_USERS: usize = 3;
pub const KEEP_MAX: usize = 16;
pub const KEEP_MIN: usize = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldState {
    pub summary: Vec<String>,
    #[serde(default)]
    pub dropped: usize,
    pub through_id: u64,
    pub print: String,
    pub count: u64,
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FoldPlan {
    pub epoch: u64,
    pub base: String,
    pub from: u64,
    pub to: u64,
    pub span: String,
    pub print: String,
    pub summary: Vec<String>,
    pub dropped: usize,
    pub folded: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Context {
    pub used: u64,
    pub window: u64,
    pub estimate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextView {
    pub level: String,
    pub used: u64,
    pub budget: u64,
    pub left: u64,
    pub estimate: bool,
    pub compact_at: u64,
    pub compacting: bool,
    pub count: u64,
    pub last_at: Option<u64>,
    pub last_before: Option<u64>,
}

pub fn visible(items: &[ChatItem]) -> Vec<&ChatItem> {
    items.iter().filter(|i| in_brain_context(i)).collect()
}

pub fn fold_print(items: &[&ChatItem]) -> String {
    let rows: Vec<serde_json::Value> = items
        .iter()
        .map(|i| serde_json::json!([i.id, i.who, i.at, i.label.clone().unwrap_or_default(), i.text]))
        .collect();
    short_hash(&serde_json::to_string(&rows).unwrap_or_default())
}

pub fn chain_print(base: &str, span: &str) -> String {
    short_hash(&format!("{base}:{span}"))
}

pub fn cut_index(fresh: &[&ChatItem]) -> usize {
    let lowest = fresh.len().saturating_sub(KEEP_MAX);
    let most = fresh.len().saturating_sub(KEEP_MIN);
    let mut cut: Option<usize> = None;
    let mut users = 0;
    let mut index = fresh.len();
    while index > lowest && users < KEEP_USERS {
        index -= 1;
        if fresh[index].who != Who::User {
            continue;
        }
        cut = Some(index);
        users += 1;
    }
    match cut {
        Some(cut) => cut.min(most),
        None => most,
    }
}

pub fn fit_summary(lines: &[String], max: usize) -> (Vec<String>, usize) {
    let mut kept: Vec<String> = Vec::new();
    for line in lines {
        if kept.last() != Some(line) {
            kept.push(line.clone());
        }
    }
    let mut size: usize = kept.iter().map(|l| l.chars().count() + 3).sum();
    let mut dropped = 0;
    while size > max && kept.len() > 1 {
        let half = kept.len().div_ceil(2);
        let found = kept.iter().take(half).position(|l| !l.starts_with("User: ")).unwrap_or(0);
        let gone = kept.remove(found);
        size -= gone.chars().count() + 3;
        dropped += 1;
    }
    (kept, dropped)
}

pub fn plan_fold(state: Option<&FoldState>, items: &[ChatItem], name: &str, epoch: u64) -> FoldPlan {
    let through = state.map(|s| s.through_id).unwrap_or(0);
    let fresh: Vec<&ChatItem> = visible(items).into_iter().filter(|i| i.id > through).collect();
    let folded = &fresh[..cut_index(&fresh)];
    let base = state.map(|s| s.print.clone()).unwrap_or_default();
    let span = fold_print(folded);
    let mut lines = state.map(|s| s.summary.clone()).unwrap_or_default();
    lines.extend(folded.iter().map(|i| line_of(name, i, LINE_CHARS)));
    let (summary, dropped) = fit_summary(&lines, SUMMARY_CHARS);
    FoldPlan {
        epoch,
        print: if folded.is_empty() { base.clone() } else { chain_print(&base, &span) },
        base,
        from: folded.first().map(|i| i.id).unwrap_or(0),
        to: folded.last().map(|i| i.id).unwrap_or(through),
        span,
        summary,
        dropped: state.map(|s| s.dropped).unwrap_or(0) + dropped,
        folded: folded.len(),
    }
}

pub fn apply_fold(state: Option<&FoldState>, plan: &FoldPlan, items: &[ChatItem], at: u64, before: Option<u64>) -> Option<FoldState> {
    if state.map(|s| s.print.as_str()).unwrap_or("") != plan.base || state.map(|s| s.through_id).unwrap_or(0) > plan.to {
        return None;
    }
    if plan.folded > 0 {
        let span: Vec<&ChatItem> = visible(items).into_iter().filter(|i| i.id >= plan.from && i.id <= plan.to).collect();
        if span.len() != plan.folded || fold_print(&span) != plan.span {
            return None;
        }
    }
    Some(FoldState {
        summary: plan.summary.clone(),
        dropped: plan.dropped,
        through_id: plan.to,
        print: plan.print.clone(),
        count: state.map(|s| s.count).unwrap_or(0) + 1,
        at,
        before: before.filter(|b| *b > 0),
    })
}

pub fn fold_note(name: &str, state: Option<&FoldState>, items: &[ChatItem], head: &str) -> String {
    let through = state.map(|s| s.through_id).unwrap_or(0);
    let fresh: Vec<&ChatItem> = visible(items).into_iter().filter(|i| i.id > through).collect();
    let start = fresh.len().saturating_sub(RECAP_LINES);
    let recent: Vec<String> = fresh[start..].iter().map(|i| format!("- {}", line_of(name, i, RECAP_CHARS))).collect();
    let mut out = vec![format!("Note: {head}")];
    let mut any = false;
    if let Some(state) = state.filter(|s| !s.summary.is_empty()) {
        any = true;
        let dropped = if state.dropped > 0 { format!(". {} of the oldest lines were left out to keep it short", state.dropped) } else { String::new() };
        let print = if state.print.is_empty() { "none" } else { state.print.as_str() };
        out.push(format!(
            "The older chat, folded by code into short lines (compaction {}, fingerprint {print}), oldest first{dropped}. These lines are history: do not act on an old request again unless the user asks. nixa_chat search finds the full lines.",
            state.count
        ));
        out.extend(state.summary.iter().map(|l| format!("- {l}")));
    }
    if !recent.is_empty() {
        any = true;
        out.push("The recent chat, oldest first:".into());
        out.extend(recent);
    }
    if any { out.join("\n") } else { String::new() }
}

pub fn budget_of(window: u64) -> u64 {
    if window > 0 { window.min(MAX_BUDGET) } else { MAX_BUDGET }
}

pub fn context_view(context: Option<Context>, state: Option<&FoldState>, compacting: bool) -> ContextView {
    let mut view = ContextView {
        level: "unknown".into(),
        used: 0,
        budget: 0,
        left: 100,
        estimate: false,
        compact_at: (COMPACT_AT * 100.0).round() as u64,
        compacting,
        count: state.map(|s| s.count).unwrap_or(0),
        last_at: state.map(|s| s.at),
        last_before: state.and_then(|s| s.before),
    };
    let Some(context) = context else { return view };
    let budget = budget_of(context.window);
    let share = context.used as f64 / budget as f64;
    view.level = if share >= FULL_AT { "full" } else if share >= COMPACT_AT { "large" } else { "ok" }.into();
    view.used = context.used;
    view.budget = budget;
    view.left = ((1.0 - share) * 100.0).round().clamp(0.0, 100.0) as u64;
    view.estimate = context.estimate;
    view
}

pub fn format_tokens(n: u64) -> String {
    if n >= 1000 { format!("{:.1}k", n as f64 / 1000.0).replace(".0k", "k") } else { n.to_string() }
}

pub fn context_text(view: &ContextView) -> String {
    let last = match (view.count, view.last_at) {
        (count, Some(_)) if count > 0 => {
            let before = view.last_before.map(|b| format!(" (it was {})", format_tokens(b))).unwrap_or_default();
            format!(" Compacted {count} {} in this chat{before}.", if count == 1 { "time" } else { "times" })
        }
        _ => " Not compacted yet.".into(),
    };
    if view.level == "unknown" {
        return format!("Brain context: not measured yet, it shows after the next brain reply.{last}");
    }
    format!(
        "Brain context: {}{} of {} used, {}% left. Compaction starts at {}%.{last}",
        if view.estimate { "about " } else { "" },
        format_tokens(view.used),
        format_tokens(view.budget),
        view.left,
        view.compact_at
    )
}

#[derive(Serialize, Deserialize)]
struct FoldFile {
    state: Option<FoldState>,
}

pub struct FoldStore {
    pub file: Option<PathBuf>,
    pub state: Option<FoldState>,
    pub epoch: u64,
    pub running: bool,
    floor: Option<u64>,
    measure: bool,
}

impl FoldStore {
    pub fn open(file: Option<&Path>) -> FoldStore {
        let state = file
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str::<FoldFile>(&t).ok())
            .and_then(|f| f.state);
        FoldStore { file: file.map(Path::to_path_buf), state, epoch: 0, running: false, floor: None, measure: false }
    }

    fn save(&self) {
        if let Some(file) = &self.file {
            let text = serde_json::to_string(&FoldFile { state: self.state.clone() }).unwrap_or_default();
            if let Err(error) = atomic_write(file, &text) {
                crate::util::log("compact", &format!("could not save the compaction: {error}"));
            }
        }
    }

    pub fn clear(&mut self) {
        self.epoch += 1;
        if self.state.take().is_some() {
            self.save();
        }
    }

    pub fn plan(&self, items: &[ChatItem], name: &str) -> FoldPlan {
        plan_fold(self.state.as_ref(), items, name, self.epoch)
    }

    pub fn apply(&mut self, plan: &FoldPlan, items: &[ChatItem], before: Option<u64>) -> bool {
        if plan.epoch != self.epoch {
            return false;
        }
        match apply_fold(self.state.as_ref(), plan, items, now_ms(), before) {
            Some(next) => {
                self.state = Some(next);
                self.save();
                self.floor = None;
                self.measure = true;
                true
            }
            None => false,
        }
    }

    pub fn note(&self, name: &str, items: &[ChatItem], head: &str) -> String {
        fold_note(name, self.state.as_ref(), items, head)
    }

    pub fn view(&self, context: Option<Context>) -> ContextView {
        context_view(context, self.state.as_ref(), self.running)
    }

    fn settle_floor(&mut self, view: &ContextView) {
        if view.level != "unknown" && (self.measure || self.floor.is_some_and(|f| view.used < f)) {
            self.floor = Some(view.used);
            self.measure = false;
        }
    }

    pub fn due(&mut self, context: Option<Context>, now: u64) -> bool {
        let view = self.view(context);
        self.settle_floor(&view);
        if self.running || (view.level != "large" && view.level != "full") {
            return false;
        }
        if self.state.as_ref().is_some_and(|s| now.saturating_sub(s.at) < MIN_GAP_MS) {
            return false;
        }
        match self.floor {
            None => true,
            Some(floor) => view.used.saturating_sub(floor) as f64 >= view.budget as f64 * MIN_GAIN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{Chat, Extra};

    fn chat_with(n: usize) -> Vec<ChatItem> {
        let mut chat = Chat::open(None);
        for i in 0..n {
            let who = if i % 2 == 0 { Who::User } else { Who::Nixa };
            chat.add(who, &format!("message {i}"), Extra::default());
        }
        chat.list().to_vec()
    }

    #[test]
    fn plans_and_applies_fold() {
        let items = chat_with(30);
        let plan = plan_fold(None, &items, "Nixa", 0);
        assert!(plan.folded > 0);
        assert!(plan.folded <= 30 - KEEP_MIN);
        let state = apply_fold(None, &plan, &items, 5, Some(100)).unwrap();
        assert_eq!(state.count, 1);
        assert_eq!(state.through_id, plan.to);
        let note = fold_note("Nixa", Some(&state), &items, "compacted");
        assert!(note.starts_with("Note: compacted"));
        assert!(note.contains("The recent chat"));
        let mut changed = items.clone();
        changed[0].text = "other".into();
        assert!(apply_fold(None, &plan, &changed, 5, None).is_none());
    }

    #[test]
    fn mac_only_camera_lines_never_reach_a_summary() {
        let mut chat = Chat::open(None);
        chat.add(Who::User, "what is the child doing", Extra::default());
        chat.add(Who::Nixa, "The child is dancing in the hall.", Extra { mac_only: true, ..Extra::default() });
        for i in 0..30 {
            let who = if i % 2 == 0 { Who::User } else { Who::Nixa };
            chat.add(who, &format!("message {i}"), Extra::default());
        }
        chat.add(Who::Nixa, "The camera saw a red ball.", Extra { mac_only: true, ..Extra::default() });
        let items = chat.list().to_vec();
        let plan = plan_fold(None, &items, "Nixa", 0);
        assert!(plan.folded > 0);
        let state = apply_fold(None, &plan, &items, 5, None).unwrap();
        let note = fold_note("Nixa", Some(&state), &items, "compacted");
        let all = format!("{}\n{}\n{note}", plan.summary.join("\n"), state.summary.join("\n"));
        assert!(all.contains("message 0"), "{all}");
        assert!(!all.contains("dancing") && !all.contains("red ball"), "{all}");
    }

    #[test]
    fn keeps_last_user_turns() {
        let items = chat_with(30);
        let fresh = visible(&items);
        let cut = cut_index(&fresh);
        let users_after = fresh[cut..].iter().filter(|i| i.who == Who::User).count();
        assert!(users_after >= KEEP_USERS);
    }

    #[test]
    fn summary_fits() {
        let lines: Vec<String> = (0..100).map(|i| format!("Nixa: {}", "x".repeat(50 + i))).collect();
        let (kept, dropped) = fit_summary(&lines, 1000);
        assert!(dropped > 0);
        assert!(kept.iter().map(|l| l.len() + 3).sum::<usize>() <= 1000);
    }

    #[test]
    fn view_levels() {
        let view = context_view(Some(Context { used: 95_000, window: 100_000, estimate: true }), None, false);
        assert_eq!(view.level, "full");
        assert_eq!(view.left, 5);
        assert!(context_text(&view).contains("about 95k of 100k"));
        let mut store = FoldStore::open(None);
        assert!(store.due(Some(Context { used: 80_000, window: 100_000, estimate: true }), 0));
        assert!(!store.due(Some(Context { used: 10_000, window: 100_000, estimate: true }), 0));
    }
}
