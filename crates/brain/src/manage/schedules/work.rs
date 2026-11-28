use super::{fail, schedule_of};
use crate::app::App;
use crate::schedules::{jobs, report, runner};
use crate::tools::{Args, ToolResult};
use nixa_store::schedules::Schedule;
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const REPORT_BYTES: u64 = 256 * 1024;
pub const RETRY_MS: u64 = 30_000;
pub const RETRY_LIMIT_MS: u64 = 30 * 60 * 1000;

pub enum Later {
    Work { task: Option<String>, hard_stop: bool },
    Run,
}

pub(super) fn blocked_now(app: &App, schedule: &Schedule) -> Option<String> {
    let spaces = app.spaces();
    let space = crate::spaces::for_folder(&spaces, &schedule.folder);
    let hold_at = crate::modes::work_now(app).hold.map(|h| h.at);
    let held = crate::term_tools::hold_stops(hold_at, crate::modes::passed(app), "schedule-run", crate::util::now_ms());
    crate::teams::runs::locked(app, space, &schedule.folder).or_else(|| held.then(|| "Hold is on.".to_string()))
}

pub fn late_text(step: &str, reason: &str) -> String {
    format!("Event: Schedule work waited {} minutes: {step}. {reason} It was queued anyway so the work is not lost. Check it with the task list; do not ask the user.", RETRY_LIMIT_MS / 60_000)
}

fn start_now(app: &Arc<App>, schedule: Schedule, later: &Later, step: &str) -> Result<(), String> {
    let folder = schedule.folder.clone();
    match later {
        Later::Work { task, hard_stop } => {
            jobs::queue_report_work(app, schedule, task.clone(), None).map_err(fail)?;
            jobs::changed(app);
            journal_start(app, step, &folder, *hard_stop);
        }
        Later::Run => {
            jobs::command(app, "schedule.run", &json!({"id": schedule.id})).map_err(fail)?;
            crate::modes::note(app, "auto-yes", step, None, Some("Nixa ran the schedule without a yes card"));
        }
    }
    Ok(())
}

pub fn waiting_text(reason: &str) -> String {
    format!("{reason} Nixa keeps this work and starts it by itself once that clears. If it still waits after {} minutes, it is queued anyway and you get an Event.", RETRY_LIMIT_MS / 60_000)
}

pub(super) fn start_later(app: &Arc<App>, schedule: Schedule, later: Later, step: String) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut waited = 0;
        while waited < RETRY_LIMIT_MS {
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_MS)).await;
            waited += RETRY_MS;
            if blocked_now(&app, &schedule).is_none() {
                break;
            }
        }
        if let Some(reason) = blocked_now(&app, &schedule) {
            let why = format!("still blocked after {} minutes ({reason}), so it was queued anyway", RETRY_LIMIT_MS / 60_000);
            crate::modes::note(&app, "auto-yes", &step, None, Some(why.as_str()));
            crate::util::log("schedules", &format!("{step}: {why}"));
            app.info(&format!("{step}: {why}."));
            crate::agent::event(&app, &late_text(&step, &reason));
        }
        if let Err(error) = start_now(&app, schedule, &later, &step) {
            crate::util::log("schedules", &format!("the waiting schedule work did not start: {error}"));
            crate::modes::note(&app, "auto-yes", &step, None, Some(format!("the waiting work did not start: {error}").as_str()));
            crate::agent::event(&app, &format!("Event: Schedule work did not start: {step}. {error} Decide the next step yourself; do not ask the user."));
        }
    });
}

fn read_report(s: &Schedule) -> Result<String, String> {
    if runner::is_team(&s.agent) {
        return Err("Team schedules do not have a report task list.".into());
    }
    let run = s.last_run.as_ref().filter(|run| run.done_at.is_some() && !run.report.is_empty())
        .ok_or("The schedule has no completed report yet.")?;
    let folder = Path::new(&s.folder).canonicalize().map_err(|_| "The schedule folder is unavailable.")?;
    let root = folder.join(report::REPORT_DIR[0]).join(report::REPORT_DIR[1]);
    let path = Path::new(&run.report).canonicalize().map_err(|_| "The saved report is unavailable.")?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("The saved report must be a file in the schedule's docs/automations folder.".into());
    }
    let mut text = String::new();
    std::fs::File::open(path).and_then(|file| file.take(REPORT_BYTES + 1).read_to_string(&mut text))
        .map_err(|_| "The saved report could not be read as text.")?;
    if u64::try_from(text.len()).is_ok_and(|len| len > REPORT_BYTES) {
        return Err("The saved report is too large to read safely.".into());
    }
    Ok(text)
}

fn open_items(text: &str) -> Vec<&str> {
    let mut tasks = false;
    let mut fence = None;
    let mut items = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(mark) = fence {
            if line.starts_with(mark) {
                fence = None;
            }
            continue;
        }
        if let Some(mark) = ["```", "~~~"].into_iter().find(|mark| line.starts_with(mark)) {
            fence = Some(mark);
        } else if let Some(heading) = line.strip_prefix("## ") {
            if tasks {
                break;
            }
            tasks = heading == "Tasks";
        } else if tasks
            && let Some(item) = line.strip_prefix("- [ ] ").or_else(|| line.strip_prefix("* [ ] "))
            && !item.trim().is_empty()
        {
            items.push(item.trim());
        }
    }
    items
}

fn journal_start(app: &App, step: &str, folder: &str, hard_stop: bool) {
    let reason = if hard_stop {
        "A push, deploy, publish or production step starts without a yes card"
    } else {
        "The report work starts without a yes card"
    };
    let input = crate::modes::journal::DecisionInput {
        kind: "tool".into(),
        choice: step.to_string(),
        reason: Some(reason.into()),
        outcome: Some("allowed".into()),
        project: Some(folder.to_string()),
        ..Default::default()
    };
    crate::modes::record(app, input);
}

pub(super) async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    if args.keys().any(|key| !matches!(key.as_str(), "action" | "id" | "task")) {
        return Err("For work, give only the schedule id and optional task number.".into());
    }
    let number = args.get("task").map(|n| {
        n.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0)
            .ok_or("task must be a positive open-task number.")
    }).transpose()?;
    let schedule = schedule_of(app, args)?;
    let step = format!("start work on the report for schedule {}", schedule.name);
    let text = read_report(&schedule)?;
    let items = open_items(&text);
    if items.is_empty() {
        return Err("The last report has no open items in its Tasks section.".into());
    }
    let selected = match number {
        Some(n) => items.get(n - 1).ok_or_else(|| format!("There is no open task {n}. The report has {} open tasks.", items.len()))?.to_string(),
        None => items.join("\n"),
    };
    let selection = number.map_or_else(|| format!("all {} open tasks", items.len()), |n| format!("open task {n}"));
    let hard_stop = crate::risk::hard_stop_text(&selected) || crate::risk::hard_stop_text(&schedule.follow_prompt);
    if let Some(reason) = blocked_now(app, &schedule) {
        let task = number.map(|n| format!("{n} (count only open checkboxes, from the top)"));
        let id = schedule.id.clone();
        crate::modes::note(app, "auto-yes", &step, None, Some("Nixa keeps the work until the lock or hold clears"));
        start_later(app, schedule, Later::Work { task, hard_stop }, step);
        return Ok(json!({"status": "waiting", "action": "work", "id": id, "selection": selection, "reason": waiting_text(&reason)}).to_string());
    }
    if jobs::get(app, &schedule.id).map_err(fail)? != schedule || read_report(&schedule)? != text {
        return Err("The schedule or report changed. Review it again before starting work.".into());
    }
    let task = number.map(|n| format!("{n} (count only open checkboxes, from the top)"));
    let id = schedule.id.clone();
    let folder = schedule.folder.clone();
    jobs::queue_report_work(app, schedule, task, None).map_err(fail)?;
    jobs::changed(app);
    journal_start(app, &step, &folder, hard_stop);
    Ok(json!({"status": "queued", "action": "work", "id": id, "selection": selection}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_work_is_queued_and_told_to_the_brain() {
        let text = late_text("start work on the report for schedule nightly", "Hold is on.");
        assert!(text.starts_with("Event: Schedule work waited 30 minutes"), "{text}");
        assert!(text.contains("queued anyway"));
        assert!(text.to_lowercase().contains("do not ask the user"), "{text}");
        assert!(waiting_text("Hold is on.").contains("queued anyway"));
    }
}
