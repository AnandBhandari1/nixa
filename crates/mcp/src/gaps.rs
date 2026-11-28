use crate::state::State;
use nixa_store::Connection;
use nixa_store::gaps::{GapSource, ToolGap, note_tool_problem};
use serde_json::Value;
use std::path::Path;

pub const HELD_MARK: &str = "Held for you";

pub fn failed_text(reply: &Value) -> Option<String> {
    if reply.get("isError").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let text = reply.get("content")?.as_array()?.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(" ");
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn space_for(conn: &Connection, folder: &Path) -> String {
    nixa_store::spaces::list_spaces(conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.folder.trim().is_empty() && folder.starts_with(Path::new(s.folder.trim())))
        .max_by_key(|s| s.folder.trim().len())
        .map(|s| s.space_id)
        .unwrap_or_default()
}

pub fn note(state: &State, tool: &str, reply: &Value) -> Option<ToolGap> {
    let text = failed_text(reply)?;
    let conn = nixa_store::open(&state.data_dir.join(nixa_store::DB_FILE)).ok()?;
    let refused = text.starts_with(HELD_MARK);
    let space = std::env::current_dir().map(|dir| space_for(&conn, &dir)).unwrap_or_default();
    note_tool_problem(&conn, tool, &text, GapSource::Mcp, refused, &space, &state.secrets).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Mode;
    use nixa_store::gaps::{GapKind, list_gaps};
    use serde_json::json;

    fn reply(text: &str, failed: bool) -> Value {
        json!({"content": [{"type": "text", "text": text}], "isError": failed})
    }

    #[test]
    fn failed_mcp_tools_are_noted_as_gaps_or_refusals() {
        let dir = std::env::temp_dir().join(format!("nixa-mcp-gaps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = State::new(Mode::Full, dir.join("e.sock"), dir.clone(), None);
        assert!(note(&state, "nixa_term_read", &reply("m1 idle", false)).is_none());
        assert!(failed_text(&reply("  ", true)).is_none());
        let gap = note(&state, "nixa_term_read", &reply("The engine is not reachable: connection refused", true)).unwrap();
        assert_eq!((gap.kind, gap.source, gap.tool.as_str()), (GapKind::Gap, GapSource::Mcp, "nixa_term_read"));
        let again = note(&state, "nixa_term_read", &reply("The engine is not reachable: connection refused", true)).unwrap();
        assert_eq!(again.count, 2);
        let held = note(&state, "nixa_term_send", &reply("Held for you: a git push waits for your yes.", true)).unwrap();
        assert_eq!(held.kind, GapKind::Refusal);
        let conn = nixa_store::open(&dir.join(nixa_store::DB_FILE)).unwrap();
        assert_eq!(list_gaps(&conn, None).unwrap().len(), 2);
        let patch = |folder: &str| nixa_store::spaces::SpacePatch { name: Some(folder.into()), folder: Some(folder.into()), ..Default::default() };
        nixa_store::spaces::set_space(&conn, "outer", &patch("/work")).unwrap();
        nixa_store::spaces::set_space(&conn, "inner", &patch("/work/shop")).unwrap();
        assert_eq!(space_for(&conn, Path::new("/work/shop/src")), "inner");
        assert_eq!(space_for(&conn, Path::new("/work/other")), "outer");
        assert_eq!(space_for(&conn, Path::new("/workshop")), "");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
