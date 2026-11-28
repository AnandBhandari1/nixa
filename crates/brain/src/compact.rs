use crate::agent;
use crate::app::{App, lock};
use crate::compaction::{context_text, format_tokens};
use crate::util::now_ms;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Busy,
    Cleaned,
}

pub fn view_text(app: &App) -> String {
    let context = agent::context(app);
    let folds = lock(&app.folds);
    context_text(&folds.view(context))
}

pub async fn after_reply(app: &Arc<App>) {
    let context = agent::context(app);
    let due = lock(&app.folds).due(context, now_ms());
    if due && !agent::busy(app) {
        compact(app, "reply");
    }
}

pub fn compact(app: &Arc<App>, reason: &str) -> Outcome {
    if agent::busy(app) || lock(&app.folds).running {
        return Outcome::Busy;
    }
    let name = app.name();
    let before = agent::context(app).map(|c| c.used);
    let items = lock(&app.chat).list().to_vec();
    let mut folds = lock(&app.folds);
    folds.running = true;
    let plan = folds.plan(&items, &name);
    agent::reset(app);
    let done = folds.apply(&plan, &items, before);
    folds.running = false;
    if !done {
        drop(folds);
        crate::util::log("compact", &format!("chat changed while compacting ({reason}), turns cleaned"));
        return Outcome::Cleaned;
    }
    let note = folds.note(&name, &items, &format!("{name} compacted its brain chat to keep it small and fast."));
    drop(folds);
    crate::privacy::keep_seen(app, &note, None);
    {
        let mut brain = lock(&app.brain);
        brain.carry = if brain.carry.is_empty() { note } else { format!("{note}\n\n{}", brain.carry) };
    }
    let was = before.map(|b| format!(" (it was {})", format_tokens(b))).unwrap_or_default();
    let what = if plan.folded > 0 {
        format!("{} older chat {} folded into the summary", plan.folded, if plan.folded == 1 { "line" } else { "lines" })
    } else {
        "the summary kept".to_string()
    };
    app.info(&format!("{name} compacted its brain context{was}: {what}, the recent turns kept."));
    Outcome::Done
}
