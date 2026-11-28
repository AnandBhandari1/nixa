//! Read-only space digest.
use crate::app::{App, lock};
use crate::spaces::Space;
use crate::util::{clip_flat, now_ms, short_hash};
use nixa_store::tasks::{TaskFilter, TaskStatus};
use serde_json::{Value, json};

pub fn digest(app: &App, space: &Space) -> Result<Value, String> {
    let space = crate::deep_service::ensure_open(app, space)?;
    let (view, rules_file, memory_file) = {
        let deep = lock(&app.deep);
        (deep.view(Some(&space.id), now_ms()), deep.rules_file.clone(), deep.memory_file.clone())
    };
    let rules = std::fs::read_to_string(rules_file).map_err(|e| format!("Cannot read deep brain rules: {e}"))?;
    let memory = std::fs::read_to_string(memory_file).map_err(|e| format!("Cannot read deep brain memory: {e}"))?;
    let parsed: Value = serde_json::from_str(&rules).map_err(|e| format!("Invalid deep brain rules: {e}"))?;
    let rule_count = parsed["rules"].as_array().map(Vec::len).ok_or("Deep brain rules need a rules list.")?;
    let conn = nixa_store::open(&app.db_file).map_err(|e| e.to_string())?;
    let agents = nixa_store::agents::list_agents(&conn).map_err(|e| e.to_string())?;
    let agents: Vec<_> = agents.into_iter().filter(|a| !a.off && (a.space.is_empty() || a.space == space.id)).collect();
    let views = crate::custom_agents::views_now(app, &agents);
    let free: Vec<Value> = views
        .iter()
        .filter(|v| crate::custom_agents::free_now(v.state, &v.busy, v.agent.off, v.sure))
        .map(|v| json!({"id": v.agent.id, "title": clip_flat(&v.agent.title, 60), "terminal": v.agent.terminal, "unsure": v.unsure}))
        .collect();
    let busy: Vec<Value> = views
        .iter()
        .filter(|v| !v.busy.is_empty())
        .map(|v| json!({"id": v.agent.id, "title": clip_flat(&v.agent.title, 60), "terminal": v.agent.terminal, "state": v.state.as_str(), "tasks": v.busy.iter().filter_map(|b| b.task).collect::<Vec<_>>(), "why": crate::custom_agents::busy_text(&v.busy)}))
        .collect();
    let filter = TaskFilter { space: Some(space.id.clone()), status: vec![TaskStatus::Doing, TaskStatus::Review, TaskStatus::Todo], ..TaskFilter::default() };
    let tasks = nixa_store::tasks::list_tasks(&conn, &filter).map_err(|e| e.to_string())?;
    let mut groups = serde_json::Map::new();
    let mut lines = vec![
        format!("Space: {} ({}).", space.name, space.id),
        format!("Rules: {rule_count} rules, hash {}. Memory: {} lines, hash {}.", short_hash(&rules), memory.lines().filter(|l| !l.trim().is_empty()).count(), short_hash(&memory)),
        format!("Deep brains: {} enabled. Open requests: {}.", view.brains.iter().filter(|b| b.seat.on && !b.seat.gave_up).count(), view.open.len()),
    ];
    for brain in &view.brains {
        if brain.seat.on && !brain.seat.gave_up {
            lines.push(format!("  {} {}: {}{}", brain.seat.name, brain.seat.terminal.as_deref().unwrap_or("no terminal"), brain.state, if brain.busy { ", busy" } else { "" }));
        }
    }
    for ask in &view.open {
        lines.push(format!("  {}: {}", ask.id, clip_flat(&ask.question, 120)));
    }
    lines.push(format!("Custom agents: {} enabled, {} free, {} busy.", agents.len(), free.len(), busy.len()));
    for agent in &free {
        let unsure = agent["unsure"].as_str().filter(|u| !u.is_empty()).map(|u| format!(", not sure: {u}")).unwrap_or_default();
        lines.push(format!("  {} {} ({}){unsure}", agent["id"].as_str().unwrap_or(""), agent["title"].as_str().unwrap_or(""), agent["terminal"].as_str().unwrap_or("")));
    }
    for agent in &busy {
        lines.push(format!("  busy {} {} ({}): {}", agent["id"].as_str().unwrap_or(""), agent["title"].as_str().unwrap_or(""), agent["terminal"].as_str().unwrap_or(""), agent["why"].as_str().unwrap_or("")));
    }
    for status in [TaskStatus::Doing, TaskStatus::Review, TaskStatus::Todo] {
        let list: Vec<_> = tasks.iter().filter(|t| t.status == status).collect();
        let items: Vec<Value> = list.iter().take(20).map(|t| json!({"id": t.id, "title": clip_flat(&t.title, 70), "assigned": t.assigned, "terminal": t.terminal_id})).collect();
        lines.push(format!("{} ({}):", status.as_str(), list.len()));
        for item in &items {
            lines.push(format!("  #{} {}", item["id"], item["title"].as_str().unwrap_or("")));
        }
        if list.len() > items.len() {
            lines.push(format!("  ... {} more", list.len() - items.len()));
        }
        groups.insert(status.as_str().into(), json!({"total": list.len(), "items": items}));
    }
    Ok(json!({
        "space": space,
        "rules": {"count": rule_count, "hash": short_hash(&rules)},
        "memory": {"lines": memory.lines().filter(|l| !l.trim().is_empty()).count(), "hash": short_hash(&memory)},
        "brains": view.brains,
        "open": view.open,
        "agents": {"total": agents.len(), "free": free, "busy": busy},
        "tasks": groups,
        "text": lines.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_store::spaces::{SpacePatch, set_space};
    use nixa_store::tasks::{Actor, AuthorKind, TaskInput, add_task};

    #[test]
    fn digest_is_scoped_read_only_and_tracks_rules_and_memory() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        for id in ["one", "two"] {
            set_space(&conn, id, &SpacePatch { name: Some(id.into()), ..SpacePatch::default() }).unwrap();
            add_task(&conn, &TaskInput { title: format!("Job in {id}"), space_id: id.into(), space_name: id.into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &Actor::new(AuthorKind::User, "user")).unwrap();
        }
        let space = crate::deep_service::resolve(&app, Some("one"), None).unwrap();
        let first = digest(&app, &space).unwrap();
        assert!(first["rules"]["count"].as_u64().unwrap() > 0);
        assert_eq!(first["tasks"]["todo"]["total"], 1);
        assert!(first["text"].as_str().unwrap().contains("Job in one"));
        assert!(!first["text"].as_str().unwrap().contains("Job in two"));
        let memory = lock(&app.deep).memory_file.clone();
        std::fs::write(memory, "# Memory\n\nOne fact\n").unwrap();
        let next = digest(&app, &space).unwrap();
        assert_ne!(first["memory"]["hash"], next["memory"]["hash"]);
        assert_eq!(next["memory"]["lines"], 2);
        assert!(lock(&app.deep).asks.is_empty());
        assert!(lock(&app.deep).seats.is_empty());
    }

    #[test]
    fn digest_lists_an_agent_with_a_spaceless_claim_as_busy_not_free() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        set_space(&conn, "one", &SpacePatch { name: Some("one".into()), ..SpacePatch::default() }).unwrap();
        let space = crate::deep_service::resolve(&app, Some("one"), None).unwrap();
        let terminal = nixa_protocol::TermId::from_number(5);
        let info = nixa_protocol::TermInfo { id: terminal.clone(), name: String::new(), labels: Default::default(), parent: None, children: Vec::new(), cmd: vec!["claude".into()], cwd: "/tmp".into(), pid: None, running: true, exit_code: None, status: None, status_note: None, keep: false, draft: false, title: None, rows: 24, cols: 80, created_ms: 0, seq: 0 };
        lock(&app.watch).world.insert(terminal.to_string(), crate::app::Seen { info, since: 0 });
        let input = nixa_store::agents::AddAgent { title: "Developer".into(), purpose: "code developer".into(), provider: "claude".into(), space: "one".into(), ..Default::default() };
        let agent = nixa_store::agents::add_agent(&conn, &input, "test").unwrap();
        nixa_store::agents::use_agent(&conn, &agent.id, &nixa_store::agents::AgentUse { terminal: Some(terminal.to_string()), ..Default::default() }).unwrap();
        let task = add_task(&conn, &TaskInput { title: "No space job".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() }, &Actor::new(AuthorKind::User, "user")).unwrap();
        assert!(nixa_store::tasks::claim_task(&conn, task.id, &Actor::in_terminal(AuthorKind::Agent, "Developer", &terminal.to_string()), "").unwrap().ok);
        let out = digest(&app, &space).unwrap();
        assert_eq!(out["agents"]["free"], json!([]), "{}", out["text"]);
        assert_eq!(out["agents"]["busy"][0]["tasks"], json!([task.id]));
        let text = out["text"].as_str().unwrap();
        assert!(text.contains("1 enabled, 0 free, 1 busy.") && text.contains(&format!("holds task #{} (doing, no space)", task.id)), "{text}");
    }

    #[test]
    fn digest_text_leaves_out_gave_up_seats_but_keeps_their_record() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        set_space(&conn, "one", &SpacePatch { name: Some("one".into()), ..SpacePatch::default() }).unwrap();
        let space = crate::deep_service::resolve(&app, Some("one"), None).unwrap();
        {
            let mut deep = lock(&app.deep);
            let mut dead = deep.new_seat(&space, "codex", "Codex");
            dead.terminal = Some("m6".into());
            dead.gave_up = true;
            deep.seats.push(dead);
            deep.join("m5", "claude", &space, Some("claude"), 0).unwrap();
        }
        let out = digest(&app, &space).unwrap();
        let text = out["text"].as_str().unwrap();
        assert!(text.contains("Deep brains: 1 enabled.") && text.contains("Claude Code m5") && !text.contains("m6"), "{text}");
        assert!(out["brains"].as_array().unwrap().iter().any(|b| b["seat"]["id"] == "codex" || b["id"] == "codex"), "{}", out["brains"]);
    }
}
