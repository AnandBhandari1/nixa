use crate::panel;
use crate::workspace::{PANEL_KEY, PanelKind};
use nixa_store::Connection;
use serde_json::Value;
use std::collections::BTreeMap;

pub const CHAT_KEY: &str = "app.chat_open";

pub fn per_space(kind: PanelKind) -> bool {
    matches!(kind, PanelKind::Files | PanelKind::Git)
}

pub fn remembered(kind: Option<PanelKind>) -> bool {
    kind.is_none_or(per_space)
}

pub fn on_switch(now: Option<PanelKind>, saved: Option<PanelKind>) -> Option<PanelKind> {
    if now == Some(PanelKind::Nixa) {
        return now;
    }
    saved.filter(|k| per_space(*k))
}

pub fn at_start(saved: Option<PanelKind>, chat_open: bool) -> Option<PanelKind> {
    if chat_open {
        return Some(PanelKind::Nixa);
    }
    saved.filter(|k| per_space(*k))
}

pub fn chat_open(conn: Option<&Connection>) -> bool {
    conn.and_then(|c| nixa_store::settings::setting_bool(c, CHAT_KEY, false).ok())
        .unwrap_or(false)
}

pub fn save(
    conn: &Connection,
    space: Option<&str>,
    before: Option<PanelKind>,
    kind: Option<PanelKind>,
) -> anyhow::Result<()> {
    let nixa = Some(PanelKind::Nixa);
    if kind == nixa || before == nixa {
        nixa_store::settings::set_setting_bool(conn, CHAT_KEY, kind == nixa)?;
    }
    if remembered(kind)
        && let Some(space) = space
    {
        let value = serde_json::json!(kind.map(panel::word));
        nixa_store::settings::set_setting(conn, &format!("{PANEL_KEY}{space}"), &value)?;
    }
    Ok(())
}

pub fn stale_keys(saved: &BTreeMap<String, Value>, spaces: &[String]) -> Vec<String> {
    saved
        .iter()
        .filter(|(key, value)| {
            let Some(space) = key.strip_prefix(PANEL_KEY) else {
                return false;
            };
            let gone = !spaces.iter().any(|s| s == space);
            let bad = match value {
                Value::Null => false,
                Value::String(word) => !panel::from_word(word).is_some_and(per_space),
                _ => true,
            };
            gone || bad
        })
        .map(|(key, _)| key.clone())
        .collect()
}

pub fn prune(conn: &Connection, spaces: &[String]) -> anyhow::Result<usize> {
    let saved = nixa_store::settings::settings_with_prefix(conn, PANEL_KEY)?;
    let stale = stale_keys(&saved, spaces);
    for key in &stale {
        nixa_store::settings::remove_setting(conn, key)?;
    }
    Ok(stale.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nixa_chat_stays_when_the_space_changes() {
        assert_eq!(
            on_switch(Some(PanelKind::Nixa), Some(PanelKind::Files)),
            Some(PanelKind::Nixa)
        );
        assert_eq!(on_switch(Some(PanelKind::Files), None), None);
        assert_eq!(on_switch(None, Some(PanelKind::Git)), Some(PanelKind::Git));
        assert_eq!(on_switch(None, Some(PanelKind::Nixa)), None);
    }

    #[test]
    fn only_space_panels_are_saved() {
        assert!(remembered(None));
        assert!(remembered(Some(PanelKind::Files)));
        assert!(remembered(Some(PanelKind::Git)));
        assert!(!remembered(Some(PanelKind::Nixa)));
    }

    #[test]
    fn the_chat_comes_back_after_a_restart() {
        let conn = nixa_store::open_memory().unwrap();
        assert!(!chat_open(Some(&conn)));
        save(&conn, Some("s1"), None, Some(PanelKind::Files)).unwrap();
        save(
            &conn,
            Some("s1"),
            Some(PanelKind::Files),
            Some(PanelKind::Nixa),
        )
        .unwrap();
        assert!(chat_open(Some(&conn)));
        let saved = crate::workspace::saved_panel(Some(&conn), "s1");
        assert_eq!(saved, Some(PanelKind::Files));
        assert_eq!(
            at_start(saved, chat_open(Some(&conn))),
            Some(PanelKind::Nixa)
        );
        save(&conn, Some("s1"), Some(PanelKind::Nixa), None).unwrap();
        assert!(!chat_open(Some(&conn)));
        assert_eq!(crate::workspace::saved_panel(Some(&conn), "s1"), None);
        assert_eq!(at_start(Some(PanelKind::Git), false), Some(PanelKind::Git));
        nixa_store::settings::set_setting(&conn, &format!("{PANEL_KEY}s1"), &json!("blueprint")).unwrap();
        assert_eq!(crate::workspace::saved_panel(Some(&conn), "s1"), None);
        assert!(!chat_open(None));
    }

    #[test]
    fn closed_spaces_and_old_panels_are_stale() {
        let mut saved = BTreeMap::new();
        saved.insert(format!("{PANEL_KEY}s1"), json!("files"));
        saved.insert(format!("{PANEL_KEY}s2"), json!(null));
        saved.insert(format!("{PANEL_KEY}s3"), json!("blueprint"));
        saved.insert(format!("{PANEL_KEY}gone"), json!("git"));
        saved.insert(format!("{PANEL_KEY}s4"), json!("nixa"));
        saved.insert("app.panel_width".into(), json!(300));
        let spaces: Vec<String> = ["s1", "s2", "s3", "s4"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut stale = stale_keys(&saved, &spaces);
        stale.sort();
        assert_eq!(
            stale,
            vec![
                format!("{PANEL_KEY}gone"),
                format!("{PANEL_KEY}s3"),
                format!("{PANEL_KEY}s4")
            ]
        );
    }
}
