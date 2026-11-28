use super::*;
use super::settle::CheckLine;
use serde_json::json;

fn facts(changed: &[&str]) -> Facts {
    Facts { changed: changed.iter().map(|f| f.to_string()).collect(), ..Facts::default() }
}

fn answers(verdict: &str, confidence: f64, finished: f64) -> Answers {
    Answers {
        answers: vec![
            Answer::Choice { name: VERDICT.into(), choice: json!(verdict), confidence, probabilities: vec![] },
            Answer::Predicate { name: FINISHED.into(), probability: finished },
        ],
        cost: 0.0,
        input_tokens: 0,
    }
}

#[test]
fn only_readable_unfailed_changes_go_to_the_decision_model() {
    assert_eq!(why_not(&facts(&["src/a.rs"]), "No checks ran"), None);
    assert!(why_not(&facts(&[]), "x").is_some());
    assert!(why_not(&Facts { unread: true, ..facts(&["a"]) }, "x").is_some());
    assert!(why_not(&Facts { no_base: true, ..facts(&["a"]) }, "x").is_some());
    assert!(why_not(&Facts { checks: Some(CheckLine { failed: true, text: "1 failed".into() }), ..facts(&["a"]) }, "x").is_some());
    assert!(why_not(&facts(&["a"]), "The agent's own report names a failing check").is_some());
}

#[test]
fn only_a_sure_done_closes_the_task() {
    assert_eq!(close(&answers(DONE, 0.95, 0.9), 0.85), Ok(()));
    assert!(close(&answers(DONE, 0.95, 0.6), 0.85).is_err());
    assert!(close(&answers(DONE, 0.7, 0.95), 0.85).is_err());
    assert!(close(&answers(FIX, 0.99, 0.1), 0.85).is_err_and(|e| e.contains("verdict fix")));
    assert!(close(&answers(UNCLEAR, 0.99, 0.5), 0.85).is_err());
}

#[test]
fn the_input_holds_the_task_report_and_files() {
    let text = input(7, "Fix the greeting typo", "Hello is spelled Helo", "Fixed the typo in src/main.rs.", &facts(&["src/main.rs"]));
    assert!(text.starts_with(&format!("Task #7: Fix the greeting typo\nTask description: Hello is spelled Helo\nChanged files: {}\n", crate::agent::data_block("src/main.rs"))));
    assert!(text.ends_with(&format!("Agent report: {}", crate::agent::data_block("Fixed the typo in src/main.rs."))));
    assert_eq!(questions().len(), 2);
}
