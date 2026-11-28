use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const MAX_ELEMENTS: usize = 600;
pub const MAX_DEPTH: usize = 16;
pub const MAIN_NAMES: [&str; 4] = ["", "main", "nixa", "app"];
pub const MAIN_PARTS: [&str; 5] = ["sidebar", "popup", "popups", "menu", "menus"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Win {
    pub id: i64,
    pub pid: i64,
    pub app: String,
    pub title: String,
    pub bounds: Bounds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppWin {
    pub id: i64,
    pub title: String,
    pub on_screen: bool,
    pub bounds: Bounds,
}

fn num(row: &Value, key: &str) -> f64 {
    row.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn word(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

pub fn bounds_of(raw: &Value) -> Bounds {
    Bounds { x: num(raw, "X"), y: num(raw, "Y"), width: num(raw, "Width"), height: num(raw, "Height") }
}

pub fn win_of(raw: &Value) -> Win {
    Win {
        id: num(raw, "kCGWindowNumber") as i64,
        pid: num(raw, "kCGWindowOwnerPID") as i64,
        app: word(raw, "kCGWindowOwnerName"),
        title: word(raw, "kCGWindowName"),
        bounds: raw.get("kCGWindowBounds").map(bounds_of).unwrap_or_default(),
        signature: None,
    }
}

pub fn normal_layer(raw: &Value) -> bool {
    raw.get("kCGWindowLayer").and_then(Value::as_i64) == Some(0)
}

pub fn windows_of(raw: &[Value], own_pid: i64) -> Vec<Win> {
    raw.iter().filter(|r| normal_layer(r)).map(win_of).filter(|w| w.pid != own_pid && w.id > 0).collect()
}

pub fn app_windows_of(raw: &[Value], is_app: impl Fn(i64) -> bool) -> Vec<AppWin> {
    raw.iter()
        .filter(|r| normal_layer(r))
        .map(|r| (win_of(r), r.get("kCGWindowIsOnscreen").and_then(Value::as_bool).unwrap_or(false)))
        .filter(|(w, _)| w.id > 0 && is_app(w.pid))
        .map(|(w, on_screen)| AppWin { id: w.id, title: w.title, on_screen, bounds: w.bounds })
        .collect()
}

pub fn is_app_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("");
    name == "nixa-app" || path.contains("/Nixa.app/Contents/MacOS/")
}

pub fn bounds_close(a: &Bounds, b: &Bounds) -> bool {
    (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0 && (a.width - b.width).abs() < 2.0 && (a.height - b.height).abs() < 2.0
}

pub fn changed_key(seen: &Win, now: &Win) -> Option<&'static str> {
    if seen.id != now.id {
        return Some("id");
    }
    if seen.pid != now.pid {
        return Some("pid");
    }
    if seen.app != now.app {
        return Some("app");
    }
    if seen.title != now.title {
        return Some("title");
    }
    if seen.bounds != now.bounds {
        return Some("bounds");
    }
    None
}

pub fn signature(rows: &[Value]) -> String {
    let text = serde_json::to_string(rows).unwrap_or_default();
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn element_row(path: &str, role: &str, subrole: &str, label: &str, enabled: bool, value: Option<&str>, url: Option<&str>) -> (Value, bool) {
    let secret = role.to_lowercase().contains("secure") || subrole.to_lowercase().contains("secure");
    let mut row = json!({"id": path, "role": if secret { "AXSecureTextField" } else { role }, "label": label, "enabled": enabled});
    if !secret && let Some(value) = value {
        row["value"] = json!(value.chars().take(2000).collect::<String>());
    }
    if let Some(url) = url {
        row["url"] = json!(url);
    }
    (row, secret)
}

pub fn label_of(parts: &[String]) -> String {
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

pub fn plain(text: &str) -> String {
    let low = text.to_lowercase();
    let words: Vec<&str> = low.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty() && *w != "window" && *w != "windows").collect();
    words.concat()
}

pub fn is_main_name(name: &str) -> bool {
    let p = plain(name);
    MAIN_NAMES.contains(&p.as_str()) || MAIN_PARTS.contains(&p.as_str())
}

pub fn is_part_name(name: &str) -> bool {
    MAIN_PARTS.contains(&plain(name).as_str())
}

pub fn main_window(list: &[AppWin]) -> Option<&AppWin> {
    list.iter().filter(|w| w.on_screen).max_by(|a, b| (a.bounds.width * a.bounds.height).total_cmp(&(b.bounds.width * b.bounds.height)))
}

pub fn pick_app_window<'a>(list: &'a [AppWin], name: &str) -> Result<&'a AppWin, String> {
    let named: Vec<&AppWin> = list.iter().filter(|w| !plain(&w.title).is_empty()).collect();
    let wanted = plain(name);
    let exact: Vec<&AppWin> = named.iter().copied().filter(|w| plain(&w.title) == wanted).collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    if exact.is_empty() && is_main_name(name) {
        return main_window(list).ok_or_else(|| "The Nixa window is not open on screen. Nothing was captured.".to_string());
    }
    let found: Vec<&AppWin> = if !exact.is_empty() {
        exact
    } else if wanted.len() >= 3 {
        named.iter().copied().filter(|w| plain(&w.title).contains(&wanted)).collect()
    } else {
        Vec::new()
    };
    match found.len() {
        1 => Ok(found[0]),
        0 => {
            let titles: Vec<&str> = named.iter().map(|w| w.title.as_str()).collect();
            Err(format!(
                "No open Nixa window is named \"{name}\". Open Nixa windows: {}. Nothing was clicked or opened.",
                if titles.is_empty() { "none".to_string() } else { titles.join(", ") }
            ))
        }
        _ => {
            let titles: Vec<&str> = found.iter().map(|w| w.title.as_str()).collect();
            Err(format!("More than one Nixa window matches \"{name}\": {}. Use the full title.", titles.join(", ")))
        }
    }
}

pub fn order_lines(mut lines: Vec<(f64, String)>) -> String {
    lines.sort_by(|a, b| b.0.total_cmp(&a.0));
    lines.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join("\n")
}

pub fn no_grant(windows: Vec<Win>, warning: &str) -> Value {
    json!({"screenRecording": false, "text": "", "elements": [], "windows": windows, "warning": warning})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: i64, pid: i64, app: &str, title: &str, layer: i64) -> Value {
        json!({"kCGWindowNumber": id, "kCGWindowOwnerPID": pid, "kCGWindowOwnerName": app, "kCGWindowName": title, "kCGWindowLayer": layer, "kCGWindowIsOnscreen": true, "kCGWindowBounds": {"X": 10, "Y": 20, "Width": 800, "Height": 600}})
    }

    fn app_win(id: i64, title: &str, w: f64) -> AppWin {
        AppWin { id, title: title.into(), on_screen: true, bounds: Bounds { x: 0.0, y: 0.0, width: w, height: w } }
    }

    #[test]
    fn no_grant_gives_window_places_and_what_to_do() {
        let wins = windows_of(&[raw(5, 100, "Notes", "", 0)], 42);
        let out = no_grant(wins, "Turn on Screen Recording.");
        assert_eq!(out["screenRecording"], json!(false));
        assert_eq!(out["windows"][0]["app"], json!("Notes"));
        assert_eq!(out["windows"][0]["bounds"]["width"], json!(800.0));
        assert_eq!(out["warning"], json!("Turn on Screen Recording."));
        assert!(out.get("image").is_none());
    }

    #[test]
    fn lists_normal_windows_of_other_apps() {
        let list = vec![raw(5, 100, "Notes", "Shop list", 0), raw(6, 100, "Notes", "menu", 25), raw(7, 42, "nixa-brain", "", 0)];
        let wins = windows_of(&list, 42);
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0].title, "Shop list");
        assert_eq!(wins[0].bounds, Bounds { x: 10.0, y: 20.0, width: 800.0, height: 600.0 });
    }

    #[test]
    fn finds_app_windows_by_owner() {
        let list = vec![raw(5, 100, "Notes", "Shop", 0), raw(8, 200, "Nixa", "Tasks", 0)];
        let wins = app_windows_of(&list, |pid| pid == 200);
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0].id, 8);
        assert!(is_app_path("/Users/someone/nixa/target/debug/nixa-app"));
        assert!(is_app_path("/Applications/Nixa.app/Contents/MacOS/nixa"));
        assert!(!is_app_path("/Applications/Other.app/Contents/MacOS/Other"));
        assert!(!is_app_path("/usr/bin/nixa-app-helper"));
    }

    #[test]
    fn picks_app_windows_by_name() {
        let list = vec![app_win(1, "Nixa", 1000.0), app_win(2, "Tasks", 400.0), app_win(3, "Agent map", 500.0)];
        assert_eq!(pick_app_window(&list, "Tasks").unwrap().id, 2);
        assert_eq!(pick_app_window(&list, "agent map window").unwrap().id, 3);
        assert_eq!(pick_app_window(&list, "main").unwrap().id, 1);
        assert_eq!(pick_app_window(&list, "sidebar").unwrap().id, 1);
        assert!(pick_app_window(&list, "Settings").unwrap_err().contains("Open Nixa windows: Nixa, Tasks, Agent map"));
        let two = vec![app_win(1, "Agent map one", 10.0), app_win(2, "Agent map two", 10.0)];
        assert!(pick_app_window(&two, "agent map").unwrap_err().contains("More than one"));
        assert!(pick_app_window(&[], "main").is_err());
    }

    #[test]
    fn hides_secret_values_and_spots_changes() {
        let (row, secret) = element_row("0.1", "AXTextField", "AXSecureTextField", "Password", true, Some("hunter"), None);
        assert!(secret);
        assert_eq!(row["role"], "AXSecureTextField");
        assert!(row.get("value").is_none());
        let (row, secret) = element_row("0.2", "AXTextField", "", "Name", true, Some("shop"), Some("https://example.com"));
        assert!(!secret);
        assert_eq!(row["value"], "shop");
        let a = win_of(&raw(5, 100, "Notes", "Shop", 0));
        let mut b = a.clone();
        assert_eq!(changed_key(&a, &b), None);
        b.title = "Other".into();
        assert_eq!(changed_key(&a, &b), Some("title"));
        assert!(bounds_close(&a.bounds, &Bounds { x: 11.0, y: 20.5, width: 800.0, height: 599.0 }));
        assert_ne!(signature(std::slice::from_ref(&row)), signature(&[]));
        assert_eq!(signature(std::slice::from_ref(&row)), signature(&[row]));
    }

    #[test]
    fn orders_text_from_the_top() {
        assert_eq!(order_lines(vec![(0.1, "bottom".into()), (0.9, "top".into()), (0.5, "middle".into())]), "top\nmiddle\nbottom");
        assert_eq!(label_of(&["Save".into(), String::new(), "Saves the file".into()]), "Save · Saves the file");
    }
}
