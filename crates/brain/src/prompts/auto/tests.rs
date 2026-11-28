use super::*;
use crate::decision::Full;
use crate::prompts::book::NewPrompt;
use crate::prompts::dialog::{Question, QuestionOption};
use nixa_store::spaces::{SpacePatch, set_space};

fn question(labels: &[&str], multi: bool) -> Question {
    Question { question: "Which database should I use?".into(), header: None, options: labels.iter().map(|l| QuestionOption { label: l.to_string(), description: None }).collect(), multi_select: multi }
}

fn prompt(kind: PromptKind, q: Option<Question>, cwd: &str) -> Prompt {
    Prompt {
        id: "p1".into(), kind, agent: "claude".into(), terminal: "m9".into(), terminal_name: None, cwd: Some(cwd.into()), place: None,
        unread: false, plain: false, text: Some("Run the tests?".into()), tool_name: Some("Bash".into()), detail: Some("cargo test".into()),
        screen_key: None, prompt_id: None, questions: q.into_iter().collect(), question_index: 0, created_at: 0, asked_at: 0,
    }
}

fn deep_app(dir: &std::path::Path) -> Arc<App> {
    let app = App::open(dir).unwrap();
    let conn = nixa_store::open(&app.db_file).unwrap();
    set_space(&conn, "shop", &SpacePatch { folder: Some(dir.display().to_string()), ..SpacePatch::default() }).unwrap();
    app.decision.set_full(Full::Deep).unwrap();
    app.prompts.set_socket(Some(dir.join("none.sock")));
    app
}

fn add(app: &App, kind: &str, q: Option<Question>, cwd: &str) -> Prompt {
    add_text(app, kind, q, cwd, "Run the tests?")
}

fn add_text(app: &App, kind: &str, q: Option<Question>, cwd: &str, text: &str) -> Prompt {
    let event = NewPrompt { kind: kind.into(), terminal: "m9".into(), agent: "claude".into(), cwd: Some(cwd.into()), text: Some(text.into()), questions: q.into_iter().collect(), ..NewPrompt::default() };
    lock(&app.prompts.book).add(event, now_ms()).unwrap()
}

fn answered_ask(app: &App, id: &str, pick: &str) -> Ask {
    let mut ask = lock(&app.deep).asks.iter().find(|a| a.id == id).cloned().unwrap();
    ask.state = AskState::Answered;
    ask.pick = Some(pick.into());
    ask.why = Some("fits the job".into());
    ask
}

#[test]
fn picks_become_answers() {
    let p = prompt(PromptKind::Permission, None, "/tmp");
    assert_eq!(answer_of_pick(&p, "Yes, allow once").unwrap().choice, Some(Choice::Yes));
    assert_eq!(answer_of_pick(&p, "2").unwrap().choice, Some(Choice::No));
    assert_eq!(answer_of_pick(&p, "deny it").unwrap().choice, Some(Choice::No));
    assert!(answer_of_pick(&p, "maybe later").is_err());
    assert!(answer_of_pick(&p, "Ask the user first").unwrap_err().contains("ask the user"));
    let plan = prompt(PromptKind::Plan, None, "/tmp");
    assert_eq!(answer_of_pick(&plan, "Approve the plan").unwrap().choice, Some(Choice::Yes));
    assert_eq!(answer_of_pick(&plan, "Reject the plan").unwrap().choice, Some(Choice::No));
    let single = prompt(PromptKind::Question, Some(question(&["SQLite", "Postgres"], false)), "/tmp");
    assert_eq!(answer_of_pick(&single, "Postgres").unwrap().options, vec![2]);
    assert_eq!(answer_of_pick(&single, "1) SQLite").unwrap().options, vec![1]);
    assert_eq!(answer_of_pick(&single, "postgres, it scales").unwrap().options, vec![2]);
    assert!(answer_of_pick(&single, "3").is_err());
    assert!(answer_of_pick(&single, "2) SQLite").is_err());
    assert!(answer_of_pick(&single, "MySQL").is_err());
    assert!(answer_of_pick(&single, "SQLite and Postgres").is_err());
    assert_eq!(answer_of_pick(&single, "text: MySQL").unwrap().text.as_deref(), Some("MySQL"));
    let multi = prompt(PromptKind::Question, Some(question(&["Tests", "Docs", "Lint"], true)), "/tmp");
    assert_eq!(answer_of_pick(&multi, "Tests, Lint").unwrap().options, vec![1, 3]);
    assert_eq!(answer_of_pick(&multi, "Docs and Docs").unwrap().options, vec![2]);
    assert!(answer_of_pick(&multi, "Tests, Deploy").is_err());
    let mut screen = single.clone();
    screen.screen_key = Some("Which database?|SQLite|Postgres".into());
    assert!(answer_of_pick(&screen, "text: MySQL").unwrap_err().contains("no typed answer"));
    screen.screen_key = Some("Which database?|SQLite|Postgres|Type something.".into());
    assert!(answer_of_pick(&screen, "text: MySQL").is_ok());
    let mut plain = single.clone();
    plain.plain = true;
    assert_eq!(answer_of_pick(&plain, "yes").unwrap().choice, Some(Choice::Yes));
    assert_eq!(answer_of_pick(&plain, "only the fast ones").unwrap().text.as_deref(), Some("only the fast ones"));
}

#[test]
fn the_ask_lists_every_option_and_the_prompt_id() {
    let mut p = prompt(PromptKind::Question, Some(question(&["SQLite", "Postgres"], true)), "/tmp");
    p.questions.push(Question { question: "Name?".into(), ..Question::default() });
    let input = ask_input(&p);
    assert_eq!(input.kind.as_deref(), Some(ASK_KIND));
    assert_eq!(input.options, vec!["SQLite".to_string(), "Postgres".to_string()]);
    assert!(input.question.contains("p1") && input.question.contains("m9") && input.question.contains("question 1 of 2"), "{}", input.question);
    assert!(input.notes.contains("joined with commas") && input.notes.contains("text: <your words>"), "{}", input.notes);
    assert!(input.notes.contains("Never pick \"ask the user\""), "{}", input.notes);
    assert_eq!(input.wait, Some(PROMPT_WAIT_MS));
    assert_eq!(ask_input(&prompt(PromptKind::Permission, None, "/tmp")).options, PERMISSION_PICKS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
}

#[test]
fn signatures_follow_the_question_not_the_id() {
    let a = prompt(PromptKind::Question, Some(question(&["SQLite", "Postgres"], false)), "/tmp");
    let mut b = a.clone();
    b.id = "p7".into();
    b.screen_key = Some("x".into());
    assert_eq!(prompt_sig(&a), prompt_sig(&b));
    let c = prompt(PromptKind::Question, Some(question(&["SQLite", "MySQL"], false)), "/tmp");
    assert_ne!(prompt_sig(&a), prompt_sig(&c));
}

#[tokio::test]
async fn decision_automatic_sends_actionable_prompts_to_the_deep_brain() {
    let dir = tempfile::tempdir().unwrap();
    let app = deep_app(dir.path());
    let folder = dir.path().display().to_string();
    let p = add(&app, "question", Some(question(&["SQLite", "Postgres"], false)), &folder);
    let ask = route_now(&app, &p).expect("routed");
    let made = lock(&app.deep).asks.iter().find(|a| a.id == ask).cloned().unwrap();
    assert_eq!(made.kind.as_deref(), Some(ASK_KIND));
    assert_eq!(made.options, vec!["SQLite".to_string(), "Postgres".to_string()]);
    assert_eq!(route_now(&app, &p), Some(ask.clone()), "the same prompt is asked once");
    assert_eq!(lock(&app.deep).asks.len(), 1);
    assert_eq!(deciding(&app, &p), Some(ask.clone()));
    let link = app.prompts.link();
    let refused = answer_by_tool(&app, &link, &p.id, Answer { options: vec![1], ..Answer::default() }, true, Some("x".into())).await.unwrap_err();
    assert!(refused.contains("deep brain is deciding"), "{refused}");
    let mut unread = p.clone();
    unread.unread = true;
    assert!(may_route(&app, &unread).is_err());
    let mut other = p.clone();
    other.cwd = Some("/nowhere/at/all".into());
    assert!(may_route(&app, &other).is_err());
}

#[tokio::test]
async fn user_words_and_full_mode_keep_prompts_away_from_the_deep_brain() {
    let dir = tempfile::tempdir().unwrap();
    let app = deep_app(dir.path());
    let folder = dir.path().display().to_string();
    let p = add(&app, "permission", None, &folder);
    assert!(may_route(&app, &p).is_ok());
    lock(&app.brain).last_user = Some(("ask me first, I will answer".into(), now_ms() + 1));
    assert!(may_route(&app, &p).unwrap_err().contains("user said"));
    lock(&app.brain).last_user = None;
    app.decision.set_full(Full::Nixa).unwrap();
    assert!(may_route(&app, &p).is_err());
    app.decision.set_full(Full::Deep).unwrap();
    assert!(may_route(&app, &p).is_ok());
}

#[tokio::test]
async fn a_prompt_that_clears_while_the_deep_brain_decides_gets_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = deep_app(dir.path());
    let folder = dir.path().display().to_string();
    let p = add(&app, "question", Some(question(&["SQLite", "Postgres"], false)), &folder);
    let ask = route_now(&app, &p).unwrap();
    lock(&app.prompts.book).remove(&p.id);
    let done = answered_ask(&app, &ask, "Postgres");
    assert!(answered(&app, &done).await);
    assert!(lock(&app.prompts.book).deep.is_empty());
    assert!(lock(&app.prompts.book).answered.is_empty());
    assert!(lock(&app.chat).list().iter().any(|i| i.text.contains("Nothing was sent")));
    assert!(answered(&app, &done).await, "a second answer is dropped");
    assert!(lock(&app.prompts.book).answered.is_empty());
}

#[tokio::test]
async fn a_new_prompt_id_for_the_same_question_is_matched() {
    let dir = tempfile::tempdir().unwrap();
    let app = deep_app(dir.path());
    let folder = dir.path().display().to_string();
    let p = add(&app, "question", Some(question(&["SQLite", "Postgres"], false)), &folder);
    let ask = route_now(&app, &p).unwrap();
    let again = add_text(&app, "question", Some(question(&["SQLite", "Postgres"], false)), &folder, "Pick a database");
    assert_ne!(again.id, p.id);
    let wait = lock(&app.prompts.book).deep[0].clone();
    assert_eq!(current(&app, &wait).map(|c| c.id), Some(again.id.clone()));
    assert_eq!(route_now(&app, &again), Some(ask.clone()));
    let changed = add_text(&app, "question", Some(question(&["Redis", "Kafka"], false)), &folder, "Pick a queue");
    assert_eq!(lock(&app.prompts.book).items.len(), 1);
    assert_eq!(lock(&app.prompts.book).items[0].id, changed.id);
    assert!(current(&app, &wait).is_none());
}

#[tokio::test]
async fn unreadable_picks_are_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let app = deep_app(dir.path());
    let folder = dir.path().display().to_string();
    let p = add(&app, "question", Some(question(&["SQLite", "Postgres"], false)), &folder);
    let ask = route_now(&app, &p).unwrap();
    assert!(answered(&app, &answered_ask(&app, &ask, "MySQL")).await);
    assert!(lock(&app.prompts.book).get(&p.id).is_some());
    assert!(lock(&app.prompts.book).answered.is_empty());
    assert!(lock(&app.prompts.book).sending.is_none());
}

#[tokio::test]
async fn other_deep_answers_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let app = deep_app(dir.path());
    let folder = dir.path().display().to_string();
    let p = add(&app, "question", Some(question(&["SQLite", "Postgres"], false)), &folder);
    let ask = route_now(&app, &p).unwrap();
    let mut other = answered_ask(&app, &ask, "Postgres");
    other.kind = Some("check".into());
    assert!(!answered(&app, &other).await);
    assert_eq!(lock(&app.prompts.book).deep.len(), 1);
    let mut late = answered_ask(&app, &ask, "Postgres");
    late.state = AskState::Timeout;
    assert!(answered(&app, &late).await, "a prompt timeout is handled by the prompt router");
    assert!(deciding(&app, &p).is_none());
    assert!(lock(&app.prompts.book).untold.contains(&p.id), "retry when a chat brain becomes available");
}
