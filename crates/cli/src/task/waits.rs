use super::caller;
use super::{open, print, route_locked, task_id};
use crate::args::usage;
use crate::ctx::{self, Ctx};
use anyhow::Result;
use nixa_store::task_rules;
use nixa_store::task_waits::{Cleared, clear_waits, open_waits, parse, set_waits, waits_of, waits_text};
use nixa_store::tasks::{self, CommentInput, CommentKind};
use serde_json::json;
use std::io::Write;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let id = task_id(&p, "wait")?;
    let clear = p.flag(&["--clear", "--none"]);
    let on = parse(&p.pos.iter().skip(1).cloned().collect::<Vec<_>>().join(" "));
    if p.pos.len() > 1 && on.is_empty() {
        return Err(usage("nixa task wait needs task ids, for example nixa task wait 12 7 9"));
    }
    p.finish(usize::MAX, false)?;
    let conn = open()?;
    if on.is_empty() && !clear {
        let all = waits_of(&conn, id);
        let still = open_waits(&conn, id);
        let line = if all.is_empty() {
            format!("Task #{id} waits on no task.")
        } else {
            format!("Task #{id} waits on {}. Still open: {}.", waits_text(&all), if still.is_empty() { "none".to_string() } else { waits_text(&still) })
        };
        return print(&ctx, out, &json!({"task": id, "waits": all, "open": still}), &line);
    }
    let who = caller::who(&ctx)?;
    let task = tasks::must_task(&conn, id)?;
    if let Some(terminal) = who.terminal() {
        task_rules::check_held_in(&task, terminal)?;
        route_locked(&conn, &task)?;
    }
    if on.is_empty() {
        let line = match clear_waits(&conn, id)? {
            Cleared::ToTodo => format!("Task #{id} waits on no task now. It was blocked on task with no holder, so it is back in todo."),
            Cleared::ToDoing => {
                let text = "Its waits were cleared. It is back in doing. Go on with it.";
                if who.terminal() != Some(task.terminal_id.as_str()) {
                    tasks::add_comment(&conn, id, &CommentInput::new(CommentKind::Note, text), &caller::actor_of(&who))?;
                }
                format!("Task #{id} waits on no task now. It was blocked on task, so it is back in doing for {}.", task.terminal_id)
            }
            Cleared::Kept => format!("Task #{id} waits on no task now."),
        };
        return print(&ctx, out, &json!({"task": id, "waits": []}), &line);
    }
    let saved = set_waits(&conn, id, &on)?;
    let line = if saved.is_empty() {
        format!("Task #{id} waits on no task now.")
    } else {
        format!("Task #{id} now waits on {}. Auto-start skips it until they all close; then Nixa moves a task blocked on task back to todo.", waits_text(&saved))
    };
    print(&ctx, out, &json!({"task": id, "waits": saved}), &line)
}
