use crate::app::App;
use nixa_protocol::control::{self, ControlReply, ControlRequest};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const OWN_WAIT: Duration = Duration::from_secs(15);

pub fn part_note(name: &str) -> Option<String> {
    nixa_desktop::rows::is_part_name(name).then(|| format!("The {name} is drawn inside the main window, so this is the whole main window."))
}

pub fn shot_value(reply: ControlReply, capture: &str, image: &Path, name: &str) -> Result<(PathBuf, Value), String> {
    match reply {
        ControlReply::Shot { image: made, window, id, bytes } => {
            let made = PathBuf::from(made);
            let wanted = format!("{capture}.png");
            if !made.is_absolute() || made.file_name().and_then(|n| n.to_str()) != Some(wanted.as_str()) {
                return Err("The Nixa window sent back a picture with the wrong name. Nothing was kept.".into());
            }
            let mut out = json!({"image": image, "window": window, "id": id, "bytes": bytes, "own": true});
            if let Some(note) = part_note(name) {
                out["note"] = json!(note);
            }
            Ok((made, out))
        }
        ControlReply::Error { message } => Err(message),
        _ => Err("The Nixa window gave no picture.".into()),
    }
}

pub fn take_file(from: &Path, to: &Path) -> Result<(), String> {
    if from != to && std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to).map_err(|e| format!("The Nixa window picture could not be kept: {e}"))?;
        std::fs::remove_file(from).ok();
    }
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("The Nixa window picture is missing: {e}"))
}

pub async fn own_shot(app: &App, name: &str, capture: &str, image: &Path) -> Result<Value, String> {
    let socket = control::app_socket_in(&app.dir);
    let request = ControlRequest::Shot { name: name.to_string(), capture: capture.to_string() };
    let reply = tokio::task::spawn_blocking(move || control::call(&socket, &request, OWN_WAIT))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    let (made, out) = shot_value(reply, capture, image, name)?;
    take_file(&made, image)?;
    Ok(out)
}
