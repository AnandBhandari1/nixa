use super::{Answer, settings};
use crate::app::{App, lock};
use crate::chat::Who;
use crate::util::{clip, log};
use std::sync::Arc;

pub const ON_TOPIC: &str = "on_topic";
pub const SHORT_WORDS: usize = 3;
pub const REFUSAL: &str = "Sorry, I am not allowed to help with that. I only help with code, tech and your work in Nixa.";
pub const ON_TOPIC_TEXT: &str = "The user's message asks about code, software, computers or other tech, asks Nixa to do something in its app (terminals, agents, tasks, spaces, settings, voice, phone, reminders), or answers what Nixa just said. Stories, poems, jokes, songs, recipes, sports, news, weather and general chat are not tech.";

pub fn short(text: &str) -> bool {
    text.split_whitespace().count() <= SHORT_WORDS
}

pub fn input(message: &str, nixa_before: Option<&str>) -> String {
    let before = nixa_before.map(|b| format!("Nixa said before: {}\n", clip(b.trim(), 400))).unwrap_or_default();
    format!("Nixa is a voice assistant for software work on a Mac.\n{before}The user says: {}", clip(message.trim(), 1500))
}

pub fn off_topic(probability: f64, level: f64) -> bool {
    probability <= 1.0 - level
}

fn last_nixa(app: &App) -> Option<String> {
    let chat = lock(&app.chat);
    let list = chat.list();
    list.iter().rev().skip(1).take(6).find(|item| item.who == Who::Nixa).map(|item| item.text.clone())
}

pub fn guard_on(app: &App, text: &str) -> Option<settings::Settings> {
    let set = settings::load(app);
    (set.uses("topic") && !short(text)).then_some(set)
}

pub async fn refuse(app: &Arc<App>, text: &str, set: &settings::Settings) -> bool {
    let input = input(text, last_nixa(app).as_deref());
    let answers = match super::ask(app, &set.model, vec![super::text(&input)], vec![super::predicate(ON_TOPIC, ON_TOPIC_TEXT)]).await {
        Ok(a) => a,
        Err(e) => {
            log("decide", &format!("topic guard skipped: {e}"));
            return false;
        }
    };
    let Some(Answer::Predicate { probability, .. }) = answers.get(ON_TOPIC) else { return false };
    let out = off_topic(*probability, set.level);
    if out {
        log("decide", &format!("topic guard refused a message (on_topic {probability:.2}): {}", clip(text, 80)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_pass_without_a_check() {
        assert!(short("yes do it"));
        assert!(!short("tell me a long story please"));
    }

    #[test]
    fn only_a_sure_off_topic_line_is_refused() {
        assert!(off_topic(0.05, 0.85));
        assert!(off_topic(0.15, 0.85));
        assert!(!off_topic(0.3, 0.85));
        assert!(!off_topic(0.9, 0.85));
    }

    #[test]
    fn the_input_holds_what_nixa_said_before() {
        let text = input("tell me a story", Some("Task 4 is done."));
        assert_eq!(text, "Nixa is a voice assistant for software work on a Mac.\nNixa said before: Task 4 is done.\nThe user says: tell me a story");
    }
}
