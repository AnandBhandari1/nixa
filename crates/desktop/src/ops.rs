use crate::rows::{Win, changed_key, is_part_name, no_grant, pick_app_window, signature};
use crate::{ax, capture, list, ocr};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const TEXT_ROLES: [&str; 3] = ["AXTextField", "AXTextArea", "AXComboBox"];
const BROWSER_APPS: [&str; 5] = ["Safari", "Google Chrome", "Google Chrome Canary", "Firefox", "Microsoft Edge"];
const OPEN_BIN: &str = "/usr/bin/open";

fn clean_http_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).ok_or("Only HTTP(S) URLs without credentials are supported.")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Only HTTP(S) URLs without credentials are supported.".into());
    }
    Ok(url.to_string())
}

fn open_safari(input: &Value, image: &Path) -> Result<Value, String> {
    let raw = input.get("url").and_then(Value::as_str).unwrap_or("");
    let url = clean_http_url(raw)?;
    let status = std::process::Command::new(OPEN_BIN)
        .args(["-a", "Safari", &url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("Could not open Safari: {e}"))?;
    if !status.success() {
        return Err("Safari could not open that page.".into());
    }
    let mut found = None;
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Some(win) = list::windows().into_iter().find(|w| w.app == "Safari") {
            found = Some(win);
            break;
        }
    }
    let target = found.ok_or("Page open was requested but no Safari window was observed. Inspect windows before retrying.")?;
    let mut result = read_window(&target, image)?;
    result["requestedURL"] = json!(url);
    Ok(result)
}

fn image_of(input: &Value) -> Result<PathBuf, String> {
    let path = input.get("image").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or("The capture place is missing.")?;
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.extension().and_then(|e| e.to_str()) != Some("png") {
        return Err("The capture place must be a full .png path.".into());
    }
    Ok(path)
}

fn before_of(image: &Path) -> PathBuf {
    image.with_extension("before.png")
}

fn read_window(target: &Win, image: &Path) -> Result<Value, String> {
    capture::capture(image, Some(target.id))?;
    let text = ocr::read_text(image)?;
    let mut bound = target.clone();
    match ax::window_of(target) {
        Ok(root) => {
            let tree = ax::tree(root.0);
            bound.signature = Some(signature(&tree.rows));
            Ok(json!({"target": bound, "elements": tree.rows, "text": text, "image": image}))
        }
        Err(error) => Ok(json!({"target": bound, "elements": [], "text": text, "image": image, "accessibilityError": error})),
    }
}

fn window_without_grant(target: &Win) -> Value {
    let mut bound = target.clone();
    let mut out = json!({"screenRecording": false, "text": "", "elements": [], "warning": capture::NO_GRANT});
    match ax::window_of(target) {
        Ok(root) => {
            let tree = ax::tree(root.0);
            bound.signature = Some(signature(&tree.rows));
            out["elements"] = json!(tree.rows);
        }
        Err(error) => out["accessibilityError"] = json!(error),
    }
    out["target"] = json!(bound);
    out
}

fn shot(name: &str, image: &Path) -> Result<Value, String> {
    capture::need_access()?;
    let windows = list::app_windows();
    let found = pick_app_window(&windows, name)?.clone();
    let bytes = capture::capture(image, Some(found.id))?;
    let mut out = json!({"image": image, "window": found.title, "id": found.id, "bytes": bytes});
    if is_part_name(name) {
        out["note"] = json!(format!("The {name} is drawn inside the main window, so this is the whole main window."));
    }
    Ok(out)
}

fn read(input: &Value, image: &Path) -> Result<Value, String> {
    match input.get("window") {
        Some(Value::String(s)) if s == "screen" => {
            if !capture::screen_access() {
                return Ok(no_grant(list::windows(), capture::NO_GRANT));
            }
            capture::capture(image, None)?;
            Ok(json!({"image": image, "text": ocr::read_text(image)?, "elements": []}))
        }
        Some(Value::String(s)) if s == "nixa" => {
            let mut out = shot("main", image)?;
            out["text"] = json!(ocr::read_text(image)?);
            out["elements"] = json!([]);
            Ok(out)
        }
        Some(Value::Number(n)) => {
            let id = n.as_i64().ok_or("Bad window id.")?;
            let target = list::window(id)?;
            if input.get("tool").and_then(Value::as_str) == Some("nixa_browser") && !BROWSER_APPS.contains(&target.app.as_str()) {
                return Err("Choose a browser window.".into());
            }
            if !capture::screen_access() {
                return Ok(window_without_grant(&target));
            }
            read_window(&target, image)
        }
        _ => Err("Give a window id, screen or nixa.".into()),
    }
}

fn act(input: &Value, image: &Path) -> Result<Value, String> {
    let action = input.get("action").and_then(Value::as_str).unwrap_or("");
    let seen: Win = input.get("target").cloned().and_then(|t| serde_json::from_value(t).ok()).ok_or("The observed window is missing.")?;
    let wanted = input.get("element").cloned().unwrap_or(Value::Null);
    let element_id = wanted.get("id").and_then(Value::as_str).ok_or("The observed control is missing.")?.to_string();
    let expected = seen.signature.clone().ok_or("The observed window has no control list. Read it again.")?;
    let now = list::window(seen.id)?;
    if let Some(key) = changed_key(&seen, &now) {
        return Err(format!("The window changed during confirmation ({key}). Read again; nothing was sent."));
    }
    let root = ax::window_of(&now)?;
    let tree = ax::tree(root.0);
    if signature(&tree.rows) != expected {
        return Err("The screen or control changed during confirmation. Read again; nothing was sent.".into());
    }
    let row = tree.rows.iter().find(|r| r.get("id").and_then(Value::as_str) == Some(element_id.as_str())).cloned().ok_or("The control is gone. Read again.")?;
    let role = row.get("role").and_then(Value::as_str).unwrap_or("").to_string();
    if row.get("enabled").and_then(Value::as_bool) != Some(true) || role.contains("Secure") {
        return Err("The control is off or secret. Nothing was sent.".into());
    }
    let node = tree.nodes.iter().find(|(id, _)| *id == element_id).map(|(_, n)| n.0).ok_or("The control is gone. Read again.")?;
    let before = before_of(image);
    capture::capture(&before, Some(now.id))?;
    let before_text = ocr::read_text(&before)?;
    let again = ax::window_of(&list::window(now.id)?)?;
    if signature(&ax::tree(again.0).rows) != expected {
        return Err("The screen changed while the before picture was taken. Read again; nothing was sent.".into());
    }
    if action == "click" {
        ax::press(node)?;
    } else {
        if !TEXT_ROLES.contains(&role.as_str()) {
            return Err("Typing needs a text field.".into());
        }
        let text = input.get("text").and_then(Value::as_str).ok_or("The text is missing.")?;
        ax::set_text(node, text)?;
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    let after = list::window(now.id).and_then(|w| read_window(&w, image));
    match after {
        Ok(mut after) => {
            after["before"] = json!({"image": before, "text": before_text, "element": wanted});
            after["inputResult"] = json!(format!("Accessibility took the {action}. Check the new screen to see that it did what was meant."));
            Ok(after)
        }
        Err(error) => Ok(json!({
            "inputResult": format!("Accessibility took the {action}."),
            "beforeImage": before,
            "beforeText": before_text,
            "afterError": error,
            "warning": "Do not repeat the input blindly. List windows to see the result.",
        })),
    }
}

pub fn execute(input: &Value) -> Result<Value, String> {
    let action = input.get("action").and_then(Value::as_str).unwrap_or("");
    match action {
        "capabilities" => Ok(json!({
            "platform": "macOS",
            "capture": "screencapture",
            "text": "Apple Vision text reading",
            "input": "Accessibility press and value",
            "shell": false,
            "screenRecording": capture::screen_access(),
            "accessibility": ax::trusted(),
        })),
        "windows" => Ok(json!({"windows": list::windows()})),
        "app-windows" => Ok(json!({"screenRecording": capture::screen_access(), "windows": list::app_windows()})),
        "ocr" => {
            let image = image_of(input)?;
            Ok(json!({"image": image, "text": ocr::read_text(&image)?, "elements": []}))
        }
        "shot" => {
            let image = image_of(input)?;
            shot(input.get("name").and_then(Value::as_str).unwrap_or(""), &image)
        }
        "open" => open_safari(input, &image_of(input)?),
        "read" => read(input, &image_of(input)?),
        "click" | "type" => act(input, &image_of(input)?),
        _ => Err("This desktop step is not supported. There is no shell, script, key or file step.".into()),
    }
}
