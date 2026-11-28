use crate::deep::{Ask, AskState};
use std::collections::{HashMap, HashSet, VecDeque};

pub const DEEP_RELAY_MS: u64 = 2 * 60 * 1000;
pub const PARK_MS: u64 = 30 * 60 * 1000;
pub const PARK_MAX: usize = 40;
pub const ASK_USER: &str = "ask the user";
pub const HOLD_PICK: &str = "hold: say what blocks and the next step Nixa takes itself, then Nixa goes on with other work";
pub const CHECK_EVENT: &str = "Event: Check";
pub const INBOX_EVENT: &str = "Event: Task inbox";
pub const BLOCKER_EVENT: &str = "Event: Task blocker";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HardKind {
    Check,
    Assign,
    Blocker,
}

impl HardKind {
    pub const ALL: [HardKind; 3] = [HardKind::Check, HardKind::Assign, HardKind::Blocker];

    pub fn word(self) -> &'static str {
        match self {
            HardKind::Check => "check",
            HardKind::Assign => "assign",
            HardKind::Blocker => "blocker",
        }
    }

    pub fn parse(word: &str) -> Option<HardKind> {
        Self::ALL.into_iter().find(|k| k.word() == word)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardTurn {
    pub kind: HardKind,
    pub label: &'static str,
    pub question: String,
    pub options: Vec<String>,
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn hard_turn_of(text: &str, full: bool) -> Option<HardTurn> {
    let user_pick = if full { HOLD_PICK } else { ASK_USER };
    if text.starts_with(CHECK_EVENT) {
        let question = if full {
            "Nixa checks finished work (the event in the notes). For each job: is the work good and done, or what is the next fix? If the agent waits for a choice, make the choice and say it in the why."
        } else {
            "Nixa checks finished work (the event in the notes). For each job: is the work good and done, or what is the next fix?"
        };
        let mut options = owned(&["done: close the job", "needs a fix: say the fix in the why"]);
        if full {
            options.push("next step: say the choice or the next work in the why".into());
        }
        options.push(user_pick.into());
        return Some(HardTurn { kind: HardKind::Check, label: "the check of finished work", question: question.into(), options });
    }
    if text.starts_with(INBOX_EVENT) {
        let options = if full { owned(&["assign each task: put one line per task in the pick, like #<task> space=<space> agent=<agent id> status=todo, and say why in the why"]) } else { owned(&["leave them in the inbox for the user"]) };
        return Some(HardTurn {
            kind: HardKind::Assign,
            label: "who gets the new tasks",
            question: "New tasks wait in Nixa's inbox (the event in the notes). Who should get each task, and in which space?".into(),
            options,
        });
    }
    if text.starts_with(BLOCKER_EVENT) {
        return Some(HardTurn {
            kind: HardKind::Blocker,
            label: "how to solve a task blocker",
            question: "A task is blocked and it is Nixa's to solve (the event in the notes). What should Nixa do?".into(),
            options: owned(&["send the agent the answer or the fix: say it in the why", "pass the task to another member", user_pick]),
        });
    }
    None
}

pub fn relay_text(ask: &Ask, relay: &str, placed: Option<&str>) -> String {
    if ask.state == AskState::Answered {
        let placed = placed.map(|line| format!("\nNixa has applied the task placements of this answer: {line} Do not place a placed task again.")).unwrap_or_default();
        return format!(
            "{relay}\n\nDeep brain answer {}: {}. Why: {}{placed}\nAct on this answer with your tools. Do something else only when a fact changed since, and then say why in one sentence.",
            ask.id,
            ask.pick.as_deref().unwrap_or(""),
            ask.why.as_deref().unwrap_or("")
        );
    }
    format!("{relay}\n\nThe deep brain did not answer {} in time. Decide this one yourself.", ask.id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub kind: HardKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parked {
    pub text: String,
    pub space: String,
    pub at: u64,
}

#[derive(Debug, Default)]
pub struct HardTurns {
    parked: VecDeque<Parked>,
    back: HashSet<String>,
    waiting: VecDeque<String>,
    relays: HashMap<String, Relay>,
    checks: usize,
}

impl HardTurns {
    pub fn wanted(&mut self, text: &str, full: bool) -> Option<HardTurn> {
        if self.back.remove(text) {
            return None;
        }
        hard_turn_of(text, full)
    }

    pub fn asked(&mut self, id: &str, kind: HardKind, text: &str) {
        if kind == HardKind::Check {
            self.checks += 1;
        }
        self.relays.insert(id.to_string(), Relay { kind, text: text.to_string() });
    }

    pub fn owns(&self, id: &str) -> bool {
        self.relays.contains_key(id)
    }

    pub fn answered(&mut self, ask: &Ask, placed: Option<&str>) -> bool {
        let Some(relay) = self.relays.remove(&ask.id) else { return false };
        if relay.kind == HardKind::Check {
            self.checks = self.checks.saturating_sub(1);
        }
        self.waiting.push_back(relay_text(ask, &relay.text, placed));
        true
    }

    pub fn take(&mut self, paused: bool) -> Vec<String> {
        if paused {
            return Vec::new();
        }
        let out: Vec<String> = self.waiting.drain(..).collect();
        self.back.extend(out.iter().cloned());
        out
    }

    pub fn check_pending(&self) -> bool {
        self.checks > 0 || self.waiting.iter().any(|t| t.starts_with(CHECK_EVENT))
    }

    pub fn park(&mut self, text: &str, space: &str, now: u64) -> Option<String> {
        if self.parked.iter().any(|p| p.text == text) {
            return None;
        }
        self.parked.push_back(Parked { text: text.to_string(), space: space.to_string(), at: now });
        if self.parked.len() > PARK_MAX {
            return self.parked.pop_front().map(|p| self.release(p.text));
        }
        None
    }

    pub fn parked_spaces(&self) -> HashSet<String> {
        self.parked.iter().map(|p| p.space.clone()).collect()
    }

    pub fn unpark(&mut self, ready: &HashSet<String>, now: u64) -> (Vec<Parked>, Vec<String>) {
        let mut send = Vec::new();
        let mut late = Vec::new();
        for parked in std::mem::take(&mut self.parked) {
            if ready.contains(&parked.space) {
                send.push(parked);
            } else if now.saturating_sub(parked.at) >= PARK_MS {
                late.push(self.release(parked.text));
            } else {
                self.parked.push_back(parked);
            }
        }
        (send, late)
    }

    pub fn release(&mut self, text: String) -> String {
        self.back.insert(text.clone());
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(id: &str, state: AskState, pick: Option<&str>, why: Option<&str>) -> Ask {
        Ask {
            id: id.into(),
            question: "q".into(),
            options: vec![],
            notes: String::new(),
            at: 0,
            state,
            tried: vec![],
            kind: Some("check".into()),
            wait: None,
            space: "s1".into(),
            space_name: "Home".into(),
            want: None,
            brain: None,
            terminal: None,
            sent_at: None,
            done_at: None,
            nudged: false,
            pick: pick.map(str::to_string),
            why: why.map(str::to_string),
            learn: None,
        }
    }

    fn full_inbox_option() -> String {
        hard_turn_of("Event: Task inbox: 1 new", true).unwrap().options.join(" ")
    }

    #[test]
    fn checks_wait_for_a_deep_brain_and_go_back_late() {
        let mut turns = HardTurns::default();
        assert_eq!(turns.park("Event: Check a", "s1", 0), None);
        assert_eq!(turns.park("Event: Check a", "s1", 0), None);
        turns.park("Event: Check b", "s2", 0);
        let none = HashSet::new();
        assert_eq!(turns.unpark(&none, 1000), (vec![], vec![]));
        let ready: HashSet<String> = ["s1".to_string()].into_iter().collect();
        let (send, late) = turns.unpark(&ready, 2000);
        assert_eq!((send.len(), send[0].text.as_str(), late.len()), (1, "Event: Check a", 0));
        let (send, late) = turns.unpark(&none, PARK_MS);
        assert!(send.is_empty());
        assert_eq!(late, vec!["Event: Check b".to_string()]);
        assert!(turns.wanted("Event: Check b", true).is_none(), "a late check goes to the quick brain once");
        assert!(turns.parked_spaces().is_empty());
    }

    #[test]
    fn turns_by_event_head_and_mode() {
        let check = hard_turn_of("Event: Check finished work", false).unwrap();
        assert_eq!(check.kind, HardKind::Check);
        assert_eq!(check.options, vec!["done: close the job", "needs a fix: say the fix in the why", ASK_USER]);
        let full = hard_turn_of("Event: Check finished work", true).unwrap();
        assert_eq!(full.options.len(), 4);
        assert_eq!(full.options[3], HOLD_PICK);
        assert!(!full.options.iter().any(|o| o.contains("ask the user")));
        assert!(!hard_turn_of("Event: Task blocker: #4", true).unwrap().options.iter().any(|o| o.contains("ask the user")));
        assert!(full.question.ends_with("say it in the why."));
        assert_eq!(hard_turn_of("Event: Task inbox: 2 new", false).unwrap().options, vec!["leave them in the inbox for the user"]);
        assert_eq!(hard_turn_of("Event: Task inbox: 2 new", true).unwrap().options, vec!["assign each task: put one line per task in the pick, like #<task> space=<space> agent=<agent id> status=todo, and say why in the why"]);
        assert!(crate::deep::parse_placements(&full_inbox_option()).is_empty());
        assert_eq!(hard_turn_of("Event: Task blocker: #4", false).unwrap().kind, HardKind::Blocker);
        assert_eq!(hard_turn_of("Heard: hello", true), None);
        assert_eq!(HardKind::parse("assign"), Some(HardKind::Assign));
    }

    #[test]
    fn relay_round_trip_skips_its_own_text() {
        let mut hard = HardTurns::default();
        let event = "Event: Check two jobs";
        let turn = hard.wanted(event, false).unwrap();
        hard.asked("d4", turn.kind, event);
        assert!(hard.owns("d4"));
        assert!(hard.check_pending());
        assert!(hard.answered(&ask("d4", AskState::Answered, Some("done: close the job"), Some("tests pass")), None));
        assert!(!hard.answered(&ask("d4", AskState::Answered, None, None), None));
        assert!(hard.check_pending());
        assert!(hard.take(true).is_empty());
        let told = hard.take(false);
        assert_eq!(told, vec!["Event: Check two jobs\n\nDeep brain answer d4: done: close the job. Why: tests pass\nAct on this answer with your tools. Do something else only when a fact changed since, and then say why in one sentence.".to_string()]);
        assert!(!hard.check_pending());
        assert_eq!(hard.wanted(&told[0], false), None);
        assert!(hard.wanted(&told[0], false).is_some());
    }

    #[test]
    fn timeout_text_hands_it_back() {
        let text = relay_text(&ask("d9", AskState::Timeout, None, None), "Event: Task blocker: x", Some("Deep brain d9: task #4 placed."));
        assert_eq!(text, "Event: Task blocker: x\n\nThe deep brain did not answer d9 in time. Decide this one yourself.");
        let placed = relay_text(&ask("d9", AskState::Answered, Some("1 #4 agent=a6 status=todo"), Some("fits")), "Event: Task inbox: x", Some("Deep brain d9: task #4 placed."));
        assert!(placed.contains("Why: fits\nNixa has applied the task placements of this answer: Deep brain d9: task #4 placed. Do not place a placed task again.\nAct on this answer"), "{placed}");
    }
}
