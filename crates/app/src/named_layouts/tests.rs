use super::store::{self, Saved};
use crate::workspace::PanelKind;
use nixa_ui::metrics::{PANEL_MAX, PANEL_WIDTH, SIDEBAR_MIN, SIDEBAR_NARROW, SIDEBAR_WIDTH};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-layouts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn saved(panel: Option<&str>) -> Saved {
    Saved {
        sidebar_width: 300.0,
        sidebar_open_width: 300.0,
        panel_width: 420.0,
        panel: panel.map(str::to_string),
    }
}

#[test]
fn names_must_be_plain() {
    assert_eq!(store::clean_name("  office  ").as_deref(), Some("office"));
    assert_eq!(store::clean_name("wide_chat-2").as_deref(), Some("wide_chat-2"));
    assert_eq!(store::clean_name(""), None);
    assert_eq!(store::clean_name("../up"), None);
    assert_eq!(store::clean_name("a/b"), None);
    assert_eq!(store::clean_name(&"x".repeat(store::NAME_MAX + 1)), None);
}

#[test]
fn a_saved_layout_comes_back_and_shows_in_the_list() {
    let dir = dir("round");
    assert!(store::list(&dir).is_empty());
    store::write(&dir, "office", &saved(Some("files"))).expect("write");
    store::write(&dir, "Chat", &saved(Some("nixa"))).expect("write");
    std::fs::write(dir.join("notes.txt"), "x").expect("write");
    assert_eq!(store::list(&dir), ["Chat", "office"]);
    assert_eq!(store::read(&dir, "office").expect("read"), saved(Some("files")));
    store::delete(&dir, "Chat").expect("delete");
    assert_eq!(store::list(&dir), ["office"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_fields_fall_back_to_the_usual_widths() {
    let read: Saved = serde_json::from_str("{}").expect("json");
    assert_eq!(read.sidebar_width, SIDEBAR_WIDTH);
    assert_eq!(read.panel_width, PANEL_WIDTH);
    assert_eq!(read.panel, None);
}

#[test]
fn widths_are_kept_inside_the_limits() {
    let wild = Saved {
        sidebar_width: 1.0,
        sidebar_open_width: 2.0,
        panel_width: 5000.0,
        panel: Some("git".into()),
    };
    let (sidebar, open, panel_w, kind) = store::fixed(&wild);
    assert_eq!(sidebar, SIDEBAR_MIN);
    assert_eq!(open, SIDEBAR_NARROW);
    assert_eq!(panel_w, PANEL_MAX);
    assert_eq!(kind, Some(PanelKind::Git));
    let odd = saved(Some("nope"));
    assert_eq!(store::fixed(&odd).3, None);
}
