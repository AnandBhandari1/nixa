use crate::app::App;
use crate::cli_brains::door::same;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::path::Path;

pub const FILE: &str = nixa_protocol::paths::WINDOW_TOKEN_FILE;
pub const HEADER: &str = nixa_protocol::paths::WINDOW_TOKEN_HEADER;

pub fn open(dir: &Path) -> String {
    crate::cli_brains::saved_token(&dir.join(FILE), "window token")
}

pub fn allowed(app: &App, headers: &HeaderMap) -> bool {
    let sent = headers.get(HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    !app.window_token.is_empty() && same(sent, &app.window_token)
}

pub fn refused() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(json!({"ok": false, "error": "The Nixa window token is missing or wrong."})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_file_is_private_and_matches() {
        let dir = std::env::temp_dir().join(format!("nixa-wk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token = open(&dir);
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert_eq!(text, token);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(token.len() >= 32);
        std::fs::remove_dir_all(&dir).ok();
    }
}
