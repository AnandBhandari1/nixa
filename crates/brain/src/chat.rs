use crate::util::{atomic_write, clip, flat, now_ms};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CHAT_LIMIT: usize = 80;
pub const CHAT_TRIM: usize = 20;
pub const RECAP_LINES: usize = 40;
pub const RECAP_CHARS: usize = 600;
pub const REPEAT_MS: u64 = 10_000;
pub const SPOKEN_REPEAT_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    User,
    Nixa,
    Info,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatItem {
    pub id: u64,
    pub who: Who,
    pub text: String,
    pub at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub typed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quiet: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub routine: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mac_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Extra {
    pub typed: bool,
    pub quiet: bool,
    pub routine: bool,
    pub label: Option<String>,
    pub kind: Option<String>,
    pub mac_only: bool,
    pub space: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct ChatFile {
    items: Vec<ChatItem>,
}

pub fn in_brain_context(item: &ChatItem) -> bool {
    item.who != Who::Info && !item.quiet && !item.routine && !item.mac_only
        && !(item.who == Who::Nixa && crate::agent::recovery::failure_line(&item.text))
}

fn plain(text: &str) -> String {
    let lowered: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    flat(&lowered)
}

pub fn line_of(name: &str, item: &ChatItem, max: usize) -> String {
    let who = match item.who {
        Who::User => "User".to_string(),
        Who::Agent => item.label.clone().unwrap_or_else(|| "An agent".into()),
        _ => name.to_string(),
    };
    format!("{who}: {}", clip(&flat(&item.text), max))
}

pub struct Chat {
    pub file: Option<PathBuf>,
    items: Vec<ChatItem>,
    next: u64,
}

impl Chat {
    pub fn open(file: Option<&Path>) -> Chat {
        let items: Vec<ChatItem> = file
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str::<ChatFile>(&t).ok())
            .map(|f| f.items)
            .unwrap_or_default();
        let start = items.len().saturating_sub(CHAT_LIMIT);
        let items = items[start..].to_vec();
        let next = items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        Chat { file: file.map(Path::to_path_buf), items, next }
    }

    fn save(&self) {
        if let Some(file) = &self.file {
            let text = serde_json::to_string(&ChatFile { items: self.items.clone() }).unwrap_or_default();
            if let Err(error) = atomic_write(file, &text) {
                crate::util::log("chat", &format!("could not save the chat: {error}"));
            }
        }
    }

    pub fn list(&self) -> &[ChatItem] {
        &self.items
    }

    pub fn add(&mut self, who: Who, raw: &str, extra: Extra) -> Option<ChatItem> {
        if raw.trim().is_empty() {
            return None;
        }
        let space = extra.space.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let text = space.as_deref().map(|s| crate::spaces::named(s, raw)).unwrap_or_else(|| raw.to_owned());
        if who == Who::Info
            && self.items.last().is_some_and(|last| {
                last.who == Who::Info && last.text.split_whitespace().eq(text.split_whitespace())
            })
        {
            return None;
        }
        let item = ChatItem {
            id: self.next,
            who,
            text,
            at: now_ms(),
            typed: extra.typed,
            quiet: extra.quiet,
            routine: extra.routine,
            label: extra.label,
            kind: extra.kind,
            mac_only: extra.mac_only,
            space,
        };
        self.next += 1;
        self.items.push(item.clone());
        if self.items.len() > CHAT_LIMIT {
            let keep = CHAT_LIMIT + 1 - CHAT_TRIM;
            let start = self.items.len() - keep;
            self.items.drain(..start);
        }
        self.save();
        Some(item)
    }

    pub fn quiet(&mut self, ids: &[u64]) {
        if ids.is_empty() {
            return;
        }
        for item in self.items.iter_mut().filter(|i| ids.contains(&i.id)) {
            item.quiet = true;
        }
        self.save();
    }

    pub fn is_repeat(&self, raw: &str, within_ms: u64) -> bool {
        let Some(index) = self.items.iter().rposition(|i| i.who == Who::User) else { return false };
        let last = &self.items[index];
        if self.items[index + 1..].iter().any(|i| i.who == Who::Nixa) {
            return false;
        }
        now_ms().saturating_sub(last.at) <= within_ms && plain(&last.text) == plain(raw)
    }

    pub fn spoke_lately(&self, raw: &str, within_ms: u64) -> bool {
        let wanted = plain(raw);
        if wanted.is_empty() {
            return false;
        }
        let now = now_ms();
        self.items.iter().rev().take_while(|i| now.saturating_sub(i.at) <= within_ms)
            .any(|i| i.who == Who::Nixa && !i.quiet && plain(&i.text) == wanted)
    }

    pub fn recap(&self, name: &str, count: usize) -> Vec<String> {
        let visible: Vec<&ChatItem> = self.items.iter().filter(|i| in_brain_context(i)).collect();
        let start = visible.len().saturating_sub(count);
        visible[start..].iter().map(|i| line_of(name, i, RECAP_CHARS)).collect()
    }

    pub fn user_lines(&self, count: usize) -> Vec<String> {
        let users: Vec<&ChatItem> = self.items.iter().filter(|i| i.who == Who::User).collect();
        let start = users.len().saturating_sub(count);
        users[start..].iter().map(|i| flat(&i.text)).collect()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.save();
    }
}

fn bullets(lines: &[String]) -> String {
    lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
}

pub fn restart_note(name: &str, lines: &[String]) -> String {
    format!("Note: {name} restarted. The last chat lines before it were:\n{}", bullets(lines))
}

pub fn brain_note(name: &str, lines: &[String]) -> String {
    format!("Note: {name} has a new brain. The chat so far, oldest first:\n{}", bullets(lines))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_trims_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("chat.json");
        let mut chat = Chat::open(Some(&file));
        assert!(chat.add(Who::User, "  ", Extra::default()).is_none());
        for n in 0..CHAT_LIMIT + 1 {
            chat.add(Who::User, &format!("line {n}"), Extra::default());
        }
        assert_eq!(chat.list().len(), CHAT_LIMIT + 1 - CHAT_TRIM);
        let back = Chat::open(Some(&file));
        assert_eq!(back.list().len(), chat.list().len());
        let mut back = back;
        let item = back.add(Who::Nixa, "hi", Extra::default()).unwrap();
        assert_eq!(item.id, CHAT_LIMIT as u64 + 2);
    }

    #[test]
    fn a_line_from_a_space_names_its_space() {
        let mut chat = Chat::open(None);
        let item = chat.add(Who::Info, "Terminal 5 stays open.", Extra { space: Some("shop".into()), ..Extra::default() }).unwrap();
        assert_eq!((item.text.as_str(), item.space.as_deref()), ("shop: Terminal 5 stays open.", Some("shop")));
        let named = chat.add(Who::Nixa, "Agent 3 in shop is done.", Extra { space: Some("shop".into()), ..Extra::default() }).unwrap();
        assert_eq!(named.text, "Agent 3 in shop is done.");
        let plain = chat.add(Who::Nixa, "All agents are idle.", Extra { space: Some(" ".into()), ..Extra::default() }).unwrap();
        assert_eq!((plain.text.as_str(), plain.space), ("All agents are idle.", None));
        let shop = Extra { space: Some("shop".into()), ..Extra::default() };
        assert!(chat.add(Who::Info, "Done.", shop.clone()).is_some());
        assert!(chat.add(Who::Info, "Done.", shop).is_none());
    }

    #[test]
    fn keeps_full_multiline_messages_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("chat.json");
        let mut chat = Chat::open(Some(&file));
        let text = format!(
            "First line\n\n  Indented text\n{}\nFinal line",
            "界".repeat(1300)
        );
        let item = chat.add(Who::Nixa, &text, Extra::default()).unwrap();
        assert_eq!(item.text, text);
        let reloaded = Chat::open(Some(&file));
        assert_eq!(reloaded.list(), &[item]);
        assert!(!reloaded.recap("Nixa", 1)[0].contains('\n'));
        chat.add(Who::User, "Keep this\n  on one line", Extra::default());
        assert_eq!(chat.user_lines(1), ["Keep this on one line"]);
    }

    #[test]
    fn info_dedupe_and_recap() {
        let mut chat = Chat::open(None);
        chat.add(Who::Info, "x", Extra::default());
        assert!(chat.add(Who::Info, " x\n", Extra::default()).is_none());
        chat.add(Who::User, "Hello there", Extra::default());
        chat.add(Who::Agent, "done", Extra { label: Some("Agent 3".into()), ..Extra::default() });
        chat.add(Who::Nixa, "ok", Extra::default());
        assert_eq!(chat.recap("Nixa", 10), vec!["User: Hello there", "Agent 3: done", "Nixa: ok"]);
    }

    #[test]
    fn repeat_check() {
        let mut chat = Chat::open(None);
        chat.add(Who::User, "Open the tasks!", Extra::default());
        assert!(chat.is_repeat("open the tasks", REPEAT_MS));
        chat.add(Who::Nixa, "Done", Extra::default());
        assert!(!chat.is_repeat("open the tasks", REPEAT_MS));
    }

    #[test]
    fn failure_lines_stay_out_of_the_recap() {
        let mut chat = Chat::open(None);
        chat.add(Who::User, "ok", Extra::default());
        chat.add(Who::Nixa, crate::agent::recovery::NOT_RETRIED, Extra::default());
        chat.add(Who::Nixa, "My OpenCode brain failed. It stopped answering.", Extra::default());
        chat.add(Who::Nixa, "All agents are idle.", Extra::default());
        assert_eq!(chat.recap("Nixa", 10), vec!["User: ok", "Nixa: All agents are idle."]);
        assert!(restart_note("Nixa", &chat.recap("Nixa", 10)).contains("All agents are idle."));
    }

    #[test]
    fn spoken_lines_are_known_for_ten_minutes() {
        let mut chat = Chat::open(None);
        chat.add(Who::Nixa, "Codex is done.", Extra::default());
        chat.add(Who::Nixa, "Quiet note.", Extra { quiet: true, ..Extra::default() });
        assert!(chat.spoke_lately("codex is done", SPOKEN_REPEAT_MS));
        assert!(!chat.spoke_lately("Quiet note.", SPOKEN_REPEAT_MS));
        assert!(!chat.spoke_lately("Something new.", SPOKEN_REPEAT_MS));
        chat.items[0].at = now_ms() - SPOKEN_REPEAT_MS - 1_000;
        assert!(!chat.spoke_lately("Codex is done.", SPOKEN_REPEAT_MS));
    }
}
