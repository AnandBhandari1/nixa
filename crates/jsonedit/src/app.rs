use crate::brain;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use json_tree_editor::{EditKind, Editor};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusKind {
    Info,
    Ok,
    Warn,
    Error,
}

pub struct Status {
    pub kind: StatusKind,
    pub text: String,
}

pub struct App {
    pub tree: String,
    pub addr: String,
    pub data_dir: PathBuf,
    pub editor: Editor,
    pub status: Status,
    pub confirm_quit: bool,
    pub confirm_reload: bool,
    pub should_quit: bool,
}

pub fn new_editor(value: Value) -> Editor {
    let mut editor = Editor::new(value);
    let boxes = editor
        .rows()
        .iter()
        .position(|row| row.key() == Some("boxes") && row.is_container() && row.is_collapsed());
    if let Some(idx) = boxes {
        editor.select(idx);
        editor.toggle();
        editor.select(0);
    }
    editor
}

pub fn open(tree: &str, addr: &str, data_dir: &Path) -> Result<App, String> {
    let value = brain::read_tree(addr, brain::token_in(data_dir).as_deref(), tree)?;
    Ok(App {
        tree: tree.to_string(),
        addr: addr.to_string(),
        data_dir: data_dir.to_path_buf(),
        editor: new_editor(value),
        status: Status {
            kind: StatusKind::Info,
            text: format!("Loaded {tree}. Edit with Enter/e/a/d, save with Ctrl+S."),
        },
        confirm_quit: false,
        confirm_reload: false,
        should_quit: false,
    })
}

fn ctrl(key: &KeyEvent, letter: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&letter))
}

impl App {
    fn token(&self) -> Option<String> {
        brain::token_in(&self.data_dir)
    }

    fn set_status(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.status = Status {
            kind,
            text: text.into(),
        };
    }

    fn report(&mut self, result: Result<(), json_tree_editor::EditError>, done: &str) {
        match result {
            Ok(()) => self.set_status(StatusKind::Info, done),
            Err(err) => self.set_status(StatusKind::Error, err.to_string()),
        }
    }

    pub fn save(&mut self) {
        self.confirm_quit = false;
        self.confirm_reload = false;
        if let Some(kind) = self.editor.edit_kind() {
            let commit = match kind {
                EditKind::Key => self.editor.confirm_key(),
                EditKind::Value => self.editor.confirm_value(),
            };
            if let Err(err) = commit {
                self.set_status(
                    StatusKind::Error,
                    format!("Finish or fix current edit first: {err}"),
                );
                return;
            }
        }
        let value = self.editor.to_value();
        match brain::put_tree(&self.addr, self.token().as_deref(), &self.tree, &value) {
            Ok(_) => {
                self.editor.mark_clean();
                let text = format!("Saved {}.", self.tree);
                self.set_status(StatusKind::Ok, text);
            }
            Err(err) => self.set_status(
                StatusKind::Error,
                format!("Save rejected (buffer kept): {err}"),
            ),
        }
    }

    pub fn reload(&mut self) {
        self.confirm_quit = false;
        if self.editor.is_dirty() && !self.confirm_reload {
            self.confirm_reload = true;
            self.set_status(
                StatusKind::Warn,
                "Unsaved changes. Press Ctrl+R again to discard and reload, or Ctrl+S to save.",
            );
            return;
        }
        self.confirm_reload = false;
        match brain::read_tree(&self.addr, self.token().as_deref(), &self.tree) {
            Ok(value) => {
                self.editor = new_editor(value);
                let text = format!("Reloaded {} from nixa-brain.", self.tree);
                self.set_status(StatusKind::Ok, text);
            }
            Err(err) => self.set_status(
                StatusKind::Error,
                format!("Reload failed (buffer kept): {err}"),
            ),
        }
    }

    pub fn request_quit(&mut self) {
        self.confirm_reload = false;
        if self.editor.is_dirty() && !self.confirm_quit {
            self.confirm_quit = true;
            self.set_status(
                StatusKind::Warn,
                "Unsaved changes. Press q again to quit without saving, or Ctrl+S to save.",
            );
            return;
        }
        self.should_quit = true;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if ctrl(&key, 's') {
            self.save();
            return;
        }
        if ctrl(&key, 'c') {
            self.request_quit();
            return;
        }
        if self.editor.is_editing() {
            self.confirm_quit = false;
            self.confirm_reload = false;
            self.editing_key(key);
        } else {
            self.normal_key(key);
        }
    }

    fn editing_key(&mut self, key: KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let plain = !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        let result = match key.code {
            KeyCode::Esc => {
                let _ = self.editor.cancel();
                self.set_status(StatusKind::Info, "Edit cancelled.");
                return;
            }
            KeyCode::Enter if shift => self.editor.newline(),
            KeyCode::Enter => match self.editor.edit_kind() {
                Some(EditKind::Key) => self.editor.confirm_key(),
                Some(EditKind::Value) => self.editor.confirm_value(),
                None => Ok(()),
            },
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.editor.newline()
            }
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete_text(),
            KeyCode::Left => self.editor.move_text_by(-1),
            KeyCode::Right => self.editor.move_text_by(1),
            KeyCode::Up => self.editor.move_line_by(-1),
            KeyCode::Down => self.editor.move_line_by(1),
            KeyCode::Home => self.editor.move_to_line_start(),
            KeyCode::End => self.editor.move_to_line_end(),
            KeyCode::Char(c) if plain => self.editor.insert(c),
            _ => Ok(()),
        };
        if let Err(err) = result {
            self.set_status(StatusKind::Error, err.to_string());
        } else if key.code == KeyCode::Enter && !shift {
            if self.editor.edit_kind() == Some(EditKind::Value) {
                self.set_status(
                    StatusKind::Info,
                    "Key set. Enter JSON value and press Enter.",
                );
            } else if !self.editor.is_editing() {
                self.set_status(
                    StatusKind::Info,
                    "Updated in buffer. Press Ctrl+S to validate and save.",
                );
            }
        }
    }

    fn normal_key(&mut self, key: KeyEvent) {
        if ctrl(&key, 'r') {
            self.reload();
            return;
        }
        if !matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q')) {
            self.confirm_quit = false;
        }
        if !matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R')) {
            self.confirm_reload = false;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.request_quit(),
            KeyCode::Char('s') | KeyCode::Char('S') => self.save(),
            KeyCode::Up | KeyCode::Char('k') => self.editor.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.editor.move_by(1),
            KeyCode::PageUp => self.editor.move_by(-10),
            KeyCode::PageDown => self.editor.move_by(10),
            KeyCode::Home | KeyCode::Char('g') => self.editor.select(0),
            KeyCode::End | KeyCode::Char('G') => {
                let count = self.editor.row_count();
                if count > 0 {
                    self.editor.select(count - 1);
                }
            }
            KeyCode::Char(' ') => self.editor.toggle(),
            KeyCode::Right | KeyCode::Char('l') => self.fold(true),
            KeyCode::Left | KeyCode::Char('h') => self.fold(false),
            KeyCode::Enter => {
                let container = self
                    .editor
                    .selected_row()
                    .map(|row| row.is_container())
                    .unwrap_or(false);
                if container {
                    self.editor.toggle();
                } else {
                    let result = self.editor.begin_value();
                    self.report(
                        result,
                        "Editing value as JSON (strings in quotes). Enter confirms, Esc cancels.",
                    );
                }
            }
            KeyCode::Char('e') => {
                let result = self.editor.begin_value();
                self.report(
                    result,
                    "Editing value as JSON. Enter confirms, Esc cancels.",
                );
            }
            KeyCode::Char('r') => {
                let result = self.editor.begin_key();
                self.report(result, "Renaming object key. Enter confirms, Esc cancels.");
            }
            KeyCode::Char('a') => {
                let result = self.editor.add();
                self.report(
                    result,
                    "Adding inside container. Enter confirms, Esc cancels.",
                );
            }
            KeyCode::Char('o') => {
                let result = self.editor.add_after();
                self.report(result, "Adding sibling after. Enter confirms, Esc cancels.");
            }
            KeyCode::Char('O') => {
                let result = self.editor.add_before();
                self.report(
                    result,
                    "Adding sibling before. Enter confirms, Esc cancels.",
                );
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let result = self.editor.delete();
                self.report(
                    result,
                    "Deleted item in buffer. Press Ctrl+S to validate and save.",
                );
            }
            _ => {}
        }
    }

    fn fold(&mut self, open: bool) {
        let collapsed = self
            .editor
            .selected_row()
            .filter(|row| row.is_container())
            .map(|row| row.is_collapsed());
        if collapsed == Some(open) {
            self.editor.toggle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::fake;
    use serde_json::json;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn with_ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn sample() -> Value {
        json!({
            "version": 1,
            "name": "t1",
            "title": "Test",
            "when": "manual",
            "start": "r1",
            "boxes": {"r1": {"text": "Start"}}
        })
    }

    fn app_at(addr: &str) -> App {
        App {
            tree: "t1".to_string(),
            addr: addr.to_string(),
            data_dir: std::env::temp_dir().join("mj-no-token-here"),
            editor: new_editor(sample()),
            status: Status {
                kind: StatusKind::Info,
                text: String::new(),
            },
            confirm_quit: false,
            confirm_reload: false,
            should_quit: false,
        }
    }

    fn select_key(app: &mut App, key: &str) {
        let idx = app
            .editor
            .rows()
            .iter()
            .position(|row| row.key() == Some(key))
            .unwrap();
        app.editor.select(idx);
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle_key(press(KeyCode::Char(c)));
        }
    }

    fn edit_title(app: &mut App, text: &str) {
        select_key(app, "title");
        app.handle_key(press(KeyCode::Char('e')));
        while app.editor.edit_text().is_some_and(|t| !t.is_empty()) {
            app.handle_key(press(KeyCode::End));
            app.handle_key(press(KeyCode::Backspace));
        }
        type_text(app, text);
        app.handle_key(press(KeyCode::Enter));
    }

    #[test]
    fn boxes_open_at_start() {
        let editor = new_editor(sample());
        let boxes = editor
            .rows()
            .into_iter()
            .find(|row| row.key() == Some("boxes"))
            .unwrap();
        assert!(!boxes.is_collapsed());
        assert!(editor.rows().iter().any(|row| row.key() == Some("r1")));
        assert_eq!(
            editor.selected_row().map(|row| row.key().is_none()),
            Some(true)
        );
    }

    #[test]
    fn edit_value_marks_dirty() {
        let mut app = app_at("127.0.0.1:1");
        edit_title(&mut app, "\"New\"");
        assert!(app.editor.is_dirty());
        assert!(!app.editor.is_editing());
        assert_eq!(app.editor.to_value()["title"], json!("New"));
        assert!(
            app.status.text.contains("Updated in buffer"),
            "{}",
            app.status.text
        );
    }

    #[test]
    fn plain_text_value_becomes_a_string() {
        let mut app = app_at("127.0.0.1:1");
        edit_title(&mut app, "not json");
        assert!(!app.editor.is_editing());
        assert_eq!(app.editor.to_value()["title"], json!("not json"));
    }

    #[test]
    fn esc_cancels_an_edit() {
        let mut app = app_at("127.0.0.1:1");
        select_key(&mut app, "title");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.editor.is_editing());
        type_text(&mut app, "zzz");
        app.handle_key(press(KeyCode::Esc));
        assert!(!app.editor.is_editing());
        assert_eq!(app.status.text, "Edit cancelled.");
        assert_eq!(app.editor.to_value(), sample());
        assert!(!app.editor.is_dirty());
    }

    #[test]
    fn quit_asks_once_when_dirty() {
        let mut app = app_at("127.0.0.1:1");
        app.handle_key(press(KeyCode::Char('q')));
        assert!(app.should_quit);
        let mut app = app_at("127.0.0.1:1");
        edit_title(&mut app, "\"x\"");
        app.handle_key(press(KeyCode::Char('q')));
        assert!(!app.should_quit);
        assert_eq!(app.status.kind, StatusKind::Warn);
        app.handle_key(press(KeyCode::Down));
        app.handle_key(press(KeyCode::Char('q')));
        assert!(!app.should_quit);
        app.handle_key(with_ctrl('c'));
        assert!(app.should_quit);
    }

    #[test]
    fn save_sends_buffer_and_marks_clean() {
        let (addr, seen) = fake::serve(vec![
            (200, json!({"ok": true, "tree": sample()}).to_string()),
            (
                400,
                json!({"ok": false, "error": "box r2 is missing"}).to_string(),
            ),
        ]);
        let mut app = app_at(&addr);
        edit_title(&mut app, "\"Saved title\"");
        app.handle_key(with_ctrl('s'));
        assert_eq!(app.status.kind, StatusKind::Ok, "{}", app.status.text);
        assert_eq!(app.status.text, "Saved t1.");
        assert!(!app.editor.is_dirty());
        let put = seen.recv().unwrap();
        let body: Value = serde_json::from_str(&put.body).unwrap();
        assert_eq!(body["tree"]["title"], json!("Saved title"));
        edit_title(&mut app, "\"Again\"");
        app.handle_key(press(KeyCode::Char('s')));
        assert_eq!(app.status.kind, StatusKind::Error);
        assert!(
            app.status
                .text
                .contains("Save rejected (buffer kept): box r2 is missing"),
            "{}",
            app.status.text
        );
        assert!(app.editor.is_dirty());
        assert_eq!(app.editor.to_value()["title"], json!("Again"));
    }

    #[test]
    fn save_commits_open_edit_first() {
        let (addr, seen) = fake::serve(vec![(
            200,
            json!({"ok": true, "tree": sample()}).to_string(),
        )]);
        let mut app = app_at(&addr);
        select_key(&mut app, "when");
        app.handle_key(press(KeyCode::Char('e')));
        while app.editor.edit_text().is_some_and(|t| !t.is_empty()) {
            app.handle_key(press(KeyCode::End));
            app.handle_key(press(KeyCode::Backspace));
        }
        type_text(&mut app, "\"always\"");
        app.handle_key(with_ctrl('s'));
        assert_eq!(app.status.kind, StatusKind::Ok, "{}", app.status.text);
        let body: Value = serde_json::from_str(&seen.recv().unwrap().body).unwrap();
        assert_eq!(body["tree"]["when"], json!("always"));
    }

    #[test]
    fn reload_asks_when_dirty_then_reads() {
        let fresh = json!({"version": 1, "name": "t1", "title": "Fresh", "when": "manual", "start": "r1", "boxes": {}});
        let (addr, seen) = fake::serve(vec![(200, json!({"ok": true, "tree": fresh}).to_string())]);
        let mut app = app_at(&addr);
        edit_title(&mut app, "\"Local\"");
        app.handle_key(with_ctrl('r'));
        assert_eq!(app.status.kind, StatusKind::Warn);
        assert_eq!(app.editor.to_value()["title"], json!("Local"));
        app.handle_key(with_ctrl('r'));
        assert_eq!(app.status.kind, StatusKind::Ok, "{}", app.status.text);
        assert_eq!(app.editor.to_value(), fresh);
        assert!(!app.editor.is_dirty());
        assert!(seen.recv().unwrap().head.starts_with("GET /trees/t1"));
    }

    #[test]
    fn save_and_reload_send_the_window_token() {
        let dir = std::env::temp_dir().join(format!("mj-app-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE), "first1").unwrap();
        let (addr, seen) = fake::serve(vec![
            (200, json!({"ok": true, "tree": sample()}).to_string()),
            (200, json!({"ok": true, "tree": sample()}).to_string()),
        ]);
        let mut app = app_at(&addr);
        app.data_dir = dir.clone();
        edit_title(&mut app, "\"T\"");
        app.handle_key(with_ctrl('s'));
        assert_eq!(app.status.kind, StatusKind::Ok, "{}", app.status.text);
        assert!(
            seen.recv()
                .unwrap()
                .head
                .contains("x-nixa-window-token: first1\r\n")
        );
        std::fs::write(dir.join(nixa_protocol::paths::WINDOW_TOKEN_FILE), "second2").unwrap();
        app.handle_key(with_ctrl('r'));
        assert_eq!(app.status.kind, StatusKind::Ok, "{}", app.status.text);
        assert!(
            seen.recv()
                .unwrap()
                .head
                .contains("x-nixa-window-token: second2\r\n")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reload_failure_keeps_buffer() {
        let mut app = app_at("127.0.0.1:1");
        app.handle_key(with_ctrl('r'));
        assert_eq!(app.status.kind, StatusKind::Error);
        assert!(app.status.text.starts_with("Reload failed (buffer kept)"));
        assert_eq!(app.editor.to_value(), sample());
    }

    #[test]
    fn add_rename_and_delete() {
        let mut app = app_at("127.0.0.1:1");
        select_key(&mut app, "r1");
        app.handle_key(press(KeyCode::Char('o')));
        assert!(app.editor.is_editing());
        type_text(&mut app, "r2");
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(
            app.status.text,
            "Key set. Enter JSON value and press Enter."
        );
        type_text(&mut app, "{\"text\": \"Two\"}");
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(app.editor.to_value()["boxes"]["r2"], json!({"text": "Two"}));
        select_key(&mut app, "r2");
        app.handle_key(press(KeyCode::Char('r')));
        while app.editor.edit_text().is_some_and(|t| !t.is_empty()) {
            app.handle_key(press(KeyCode::End));
            app.handle_key(press(KeyCode::Backspace));
        }
        type_text(&mut app, "r3");
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(app.editor.to_value()["boxes"]["r3"], json!({"text": "Two"}));
        select_key(&mut app, "r3");
        app.handle_key(press(KeyCode::Char('d')));
        assert!(app.editor.to_value()["boxes"].get("r3").is_none());
        assert!(app.status.text.starts_with("Deleted item in buffer"));
    }

    #[test]
    fn fold_keys_and_moves() {
        let mut app = app_at("127.0.0.1:1");
        select_key(&mut app, "boxes");
        app.handle_key(press(KeyCode::Char('h')));
        assert!(app.editor.selected_row().unwrap().is_collapsed());
        app.handle_key(press(KeyCode::Char('h')));
        assert!(app.editor.selected_row().unwrap().is_collapsed());
        app.handle_key(press(KeyCode::Right));
        assert!(!app.editor.selected_row().unwrap().is_collapsed());
        app.handle_key(press(KeyCode::Char('G')));
        let last = app.editor.rows().last().map(|row| row.kind());
        assert_eq!(app.editor.selected_row().map(|row| row.kind()), last);
        app.handle_key(press(KeyCode::Char('g')));
        assert!(app.editor.selected_row().unwrap().key().is_none());
        app.handle_key(press(KeyCode::Char('j')));
        let second = app.editor.rows()[1].key().map(str::to_string);
        assert_eq!(
            app.editor.selected_row().unwrap().key().map(str::to_string),
            second
        );
        app.handle_key(press(KeyCode::Char('k')));
        assert!(app.editor.selected_row().unwrap().key().is_none());
    }
}
