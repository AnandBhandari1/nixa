use crate::app::{App, lock};
use crate::trees::{WATCH_TREE, check_tree};
use crate::util::{log, now_ms};
use serde_json::{Value, json};

pub const MAX_MINUTES: u64 = 240;

pub fn minutes_of(message: &Value) -> Result<u64, String> {
    message
        .get("minutes")
        .and_then(Value::as_u64)
        .filter(|minutes| *minutes <= MAX_MINUTES)
        .ok_or_else(|| format!("set_watch needs minutes as a whole number from 0 to {MAX_MINUTES}."))
}

pub fn timer_line(minutes: u64) -> String {
    match minutes {
        0 => "Watch the agents: timer off.".into(),
        1 => "Watch the agents: every minute.".into(),
        n => format!("Watch the agents: every {n} minutes."),
    }
}

pub fn set_watch(app: &App, message: &Value) -> Result<Value, String> {
    let minutes = minutes_of(message)?;
    if minutes == app.watch_minutes() {
        return Ok(json!({"minutes": minutes, "changed": false}));
    }
    app.save_settings(|settings| settings.watch_minutes = Some(minutes))
        .map_err(|error| format!("Could not save the watch timer: {error}"))?;
    lock(&app.watch).last_at = now_ms();
    log("watch", &format!("timer every {minutes} minutes"));
    app.info(&timer_line(minutes));
    super::send_snapshot(app);
    Ok(json!({"minutes": minutes, "changed": true}))
}

pub fn dry_run(app: &App, message: &Value) -> Result<Value, String> {
    let raw = message.get("tree").filter(|tree| !tree.is_null());
    let tree = match raw {
        Some(value) => check_tree(value, Some(WATCH_TREE))?,
        None => lock(&app.trees).read(WATCH_TREE)?,
    };
    let text = crate::walk::dry_run(&tree, &crate::watch::subjects(app));
    let tested = raw.is_some();
    log("watch", &format!("dry run, tested {tested}, {} chars", text.len()));
    let shown = if tested { format!("{text}\n(Tested a changed tree that is not saved.)") } else { text.clone() };
    app.info(&shown);
    Ok(json!({"text": text, "tested": tested}))
}
