use crate::screens::ctxmenu::Target;
use crate::screens::editor;
use crate::workspace::Workspace;
use gpui::{Context, Pixels, Point, Window};
use nixa_protocol::TermId;
use nixa_termview::target;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Web(String),
    File { path: String, line: Option<u32> },
    Folder(String),
}

fn candidates(raw: &str, cwd: &str, home: &str) -> Vec<PathBuf> {
    let raw = raw.strip_prefix("file://").unwrap_or(raw);
    if raw == "~" {
        return vec![PathBuf::from(home)];
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return vec![Path::new(home).join(rest)];
    }
    if raw.starts_with('/') {
        return vec![PathBuf::from(raw)];
    }
    if cwd.is_empty() {
        return Vec::new();
    }
    let base = Path::new(cwd);
    let mut out = vec![base.join(raw.strip_prefix("./").unwrap_or(raw))];
    if let Some(rest) = raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/")) {
        out.push(base.join(rest));
    }
    out
}

pub fn resolve(text: &str, cwd: &str, home: &str) -> Option<Found> {
    if target::is_web(text) {
        return Some(Found::Web(text.to_string()));
    }
    let (raw, line, _) = target::split_line(text);
    let (full, meta) = candidates(raw, cwd, home)
        .into_iter()
        .find_map(|p| std::fs::metadata(&p).ok().map(|m| (p, m)))?;
    let path = full.to_string_lossy().to_string();
    Some(if meta.is_dir() {
        Found::Folder(path)
    } else {
        Found::File { path, line }
    })
}

pub fn editor_show(path: &str, line: Option<u32>) -> String {
    match line {
        Some(n) => format!("{}{path}|line:{n}", editor::SHOW),
        None => format!("{}{path}", editor::SHOW),
    }
}

pub fn click(
    ws: &mut Workspace,
    id: &TermId,
    at: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(text) = ws.views.get(id).and_then(|v| v.read(cx).link_at(at)) else {
        return false;
    };
    let cwd = ws.terms.get(id).map(|t| t.cwd.clone()).unwrap_or_default();
    match resolve(&text, &cwd, &crate::spaces::home()) {
        Some(Found::Web(url)) => cx.open_url(&url),
        Some(Found::File { path, line }) => {
            editor::show(&editor_show(&path, line), cx);
        }
        Some(Found::Folder(path)) => ws.open_ctx(Target::Link { path }, at, window, cx),
        None => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nixa-term-links-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[test]
    fn web_links_need_no_folder() {
        assert_eq!(
            resolve("https://example.com/a", "", ""),
            Some(Found::Web("https://example.com/a".into()))
        );
    }

    #[test]
    fn paths_resolve_from_the_terminal_folder() {
        let dir = temp();
        let cwd = dir.to_string_lossy().to_string();
        let file = dir.join("src").join("main.rs").to_string_lossy().to_string();
        let folder = dir.join("src").to_string_lossy().to_string();
        assert_eq!(
            resolve("src/main.rs:12:3", &cwd, "/nowhere"),
            Some(Found::File {
                path: file.clone(),
                line: Some(12)
            })
        );
        assert_eq!(
            resolve("./src/main.rs", &cwd, "/nowhere"),
            Some(Found::File {
                path: file.clone(),
                line: None
            })
        );
        assert_eq!(
            resolve("b/src/main.rs", &cwd, "/nowhere"),
            Some(Found::File {
                path: file.clone(),
                line: None
            })
        );
        assert_eq!(resolve("src", &cwd, "/nowhere"), Some(Found::Folder(folder)));
        assert_eq!(
            resolve("~/src/main.rs:4", "/nowhere", &cwd),
            Some(Found::File {
                path: file.clone(),
                line: Some(4)
            })
        );
        assert_eq!(
            resolve(&format!("{file}:2"), "", "/nowhere"),
            Some(Found::File {
                path: file,
                line: Some(2)
            })
        );
        assert_eq!(resolve("src/missing.rs", &cwd, "/nowhere"), None);
        assert_eq!(resolve("src/main.rs", "", "/nowhere"), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_editor_gets_the_line() {
        assert_eq!(editor_show("/a/b.rs", Some(7)), format!("{}/a/b.rs|line:7", editor::SHOW));
        assert_eq!(editor_show("/a/b.rs", None), format!("{}/a/b.rs", editor::SHOW));
    }
}
