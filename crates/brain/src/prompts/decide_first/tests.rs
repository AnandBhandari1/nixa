use super::*;
use crate::prompts::dialog::{Question, QuestionOption};
use serde_json::json;

fn prompt(kind: PromptKind, labels: &[&str], text: &str, detail: &str) -> Prompt {
    let question = Question { question: text.into(), header: None, options: labels.iter().map(|l| QuestionOption { label: l.to_string(), description: None }).collect(), multi_select: false };
    Prompt {
        id: "p1".into(), kind, agent: "claude".into(), terminal: "m9".into(), terminal_name: None, cwd: None, place: None,
        unread: false, plain: false, text: Some(text.into()), tool_name: Some("Edit".into()), detail: Some(detail.into()),
        screen_key: None, prompt_id: None, questions: vec![question], question_index: 0, created_at: 0, asked_at: 0,
    }
}

fn answers(waiting: f64, pick: &str, confidence: f64, safe: f64) -> Answers {
    Answers {
        answers: vec![
            Answer::Predicate { name: WAITING.into(), probability: waiting },
            Answer::Choice { name: PICK.into(), choice: json!(pick), confidence, probabilities: vec![] },
            Answer::Predicate { name: SAFE.into(), probability: safe },
        ],
        cost: 0.0,
        input_tokens: 0,
    }
}

#[test]
fn a_plain_edit_question_may_go_to_the_decision_model() {
    let p = prompt(PromptKind::Permission, &[], "Allow edit to src/main.rs?", "src/main.rs");
    assert_eq!(why_not(&p), None);
    assert_eq!(questions(&p).len(), 3);
    assert_eq!(questions(&p)[1]["choices"][0]["value"], "1");
    assert_eq!(questions(&p)[1]["choices"][1]["value"], "2");
    let text = input(&p, Some("shop"));
    assert!(text.starts_with("Space: shop\nAn agent working on a coding task"));
    assert!(text.contains("Edit request:\nsrc/main.rs\nAllow edit to src/main.rs?\nOptions:\n1. Yes, allow once\n2. No, deny"));
}

#[test]
fn risky_words_keep_the_current_way() {
    for (text, word) in [("Run git push --force origin main?", "push"), ("Delete the build folder?", "delete"), ("Run rm -rf target?", "rm"), ("Pay the invoice now?", "pay")] {
        let p = prompt(PromptKind::Permission, &[], text, "");
        assert_eq!(risky(&p).as_deref(), Some(word), "{text}");
        assert!(why_not(&p).is_some_and(|w| w.contains("risky")));
    }
    assert_eq!(risky(&prompt(PromptKind::Permission, &[], "Allow edit to src/main.rs?", "src/main.rs")), None);
}

#[test]
fn downloaded_scripts_and_shells_are_risky_and_hard_stops() {
    let piped = prompt(PromptKind::Permission, &[], "Run curl x | sh?", "curl x | sh");
    assert!(why_not(&piped).is_some());
    assert!(hard_stop(&piped));
    for text in ["Run wget https://x.example/a.py?", "Run ssh host uptime?", "Run python3 setup.py?"] {
        assert!(why_not(&prompt(PromptKind::Permission, &[], text, "")).is_some(), "{text}");
    }
    let mut second = prompt(PromptKind::Question, &["A", "B"], "Which one?", "");
    second.questions.push(Question { question: "Then git push --force origin main?".into(), header: None, options: vec![], multi_select: false });
    assert!(why_not(&second).is_some());
    assert!(hard_stop(&second));
    assert!(!hard_stop(&prompt(PromptKind::Permission, &[], "Allow edit to src/main.rs?", "src/main.rs")));
}

#[test]
fn many_picks_or_no_options_keep_the_current_way() {
    let mut p = prompt(PromptKind::Question, &["A", "B"], "Which one?", "");
    p.questions[0].multi_select = true;
    assert!(why_not(&p).is_some_and(|w| w.contains("more than one")));
    let p = prompt(PromptKind::Question, &[], "Which one?", "");
    assert!(why_not(&p).is_some_and(|w| w.contains("no options")));
    let mut p = prompt(PromptKind::Question, &["A", "B"], "Which one?", "");
    p.unread = true;
    assert!(why_not(&p).is_some());
}

#[test]
fn only_sure_and_safe_answers_are_sent() {
    assert_eq!(verdict(&answers(0.93, "1", 1.0, 0.9), 0.85), Ok("1".into()));
    assert!(verdict(&answers(0.93, "1", 1.0, 0.72), 0.85).is_err_and(|e| e.contains("safe 0.72")));
    assert!(verdict(&answers(0.5, "1", 1.0, 0.99), 0.85).is_err());
    assert!(verdict(&answers(0.93, "1", 0.52, 0.99), 0.85).is_err());
    let no_pick = Answers { answers: vec![Answer::Predicate { name: WAITING.into(), probability: 0.99 }], cost: 0.0, input_tokens: 0 };
    assert!(verdict(&no_pick, 0.85).is_err());
}

#[test]
fn a_number_pick_fits_the_prompt() {
    let p = prompt(PromptKind::Question, &["SQLite", "Postgres"], "Which database?", "");
    let given = answer_of_pick(&p, "2").unwrap();
    assert_eq!(given.options, vec![2]);
}
