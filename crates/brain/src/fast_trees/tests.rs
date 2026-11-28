use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn engine() -> Compiled {
    compile(&seed()).unwrap()
}

fn facts(list: &[(&str, bool)]) -> BTreeMap<String, bool> {
    list.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn with(change: impl FnOnce(&mut Value)) -> Result<Compiled, String> {
    let mut value = seed_value();
    change(&mut value);
    compile(&rules_of(&value)?)
}

#[test]
fn seed_compiles_and_checks() {
    let check = engine().self_check();
    assert!(check.ok, "{:?}", check.problems);
    assert_eq!((check.keywords, check.phrases, check.table), (43, 236, 88));
    assert_eq!(check.trees, vec![rules::TreeDepth { name: ROUTING.into(), depth: 5 }]);
    assert_eq!(check.trie_states, 428);
}

#[test]
fn same_answers_as_the_old_helper() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("parity.json")).unwrap();
    let engine = engine();
    assert!(cases.len() >= 40);
    for case in cases {
        let text = case["s"].as_str().unwrap();
        let held = case["held"].as_bool().unwrap();
        let got = engine.decide(text, &Input { held, ..Input::default() }).unwrap();
        assert_eq!(serde_json::to_value(&got).unwrap(), case["d"], "{text} held {held}");
    }
}

#[test]
fn phrases_clean_and_scan() {
    assert_eq!(clean_phrase("  Don’t   START, anyone! "), "dont start anyone");
    let index = PhraseIndex::new(&[("stop".into(), 0), ("stop everything".into(), 1), ("everything".into(), 2)]).unwrap();
    let (words, hits) = index.scan("Please STOP everything now");
    assert_eq!(words, vec!["please", "stop", "everything", "now"]);
    let spans: Vec<(usize, usize, usize)> = hits.iter().map(|h| (h.item, h.start, h.end)).collect();
    assert_eq!(spans, vec![(0, 1, 2), (1, 1, 3), (2, 2, 3)]);
    assert!(PhraseIndex::new(&[(" ,, ".into(), 0)]).is_err());
}

#[test]
fn trees_reject_bad_shapes() {
    let chart = |steps: Value| chart_of(&json!({"start": "a", "steps": steps})).unwrap();
    let cycle = chart(json!({"a": {"kind": "check", "key": "x", "label": "X", "yes": "b", "no": "c"}, "b": {"kind": "start", "next": "a"}, "c": {"kind": "action", "key": "k", "label": "K"}}));
    assert!(compile_tree(&cycle, 12).unwrap_err().contains("cycle"));
    let missing = chart(json!({"a": {"kind": "start", "next": "z"}}));
    assert!(compile_tree(&missing, 12).unwrap_err().contains("missing node z"));
    let lonely = chart(json!({"a": {"kind": "action", "key": "k", "label": "K"}, "b": {"kind": "action", "key": "k", "label": "K"}}));
    assert!(compile_tree(&lonely, 12).unwrap_err().contains("unreachable"));
    let deep = chart(json!({
        "a": {"kind": "check", "key": "x", "label": "X", "yes": "b", "no": "z"},
        "b": {"kind": "check", "key": "y", "label": "Y", "yes": "z", "no": "z"},
        "z": {"kind": "action", "key": "k", "label": "K"}
    }));
    assert!(compile_tree(&deep, 1).unwrap_err().contains("exceeds limit 1"));
    let tree = compile_tree(&deep, 2).unwrap();
    assert_eq!(tree.depth, 2);
    assert_eq!(tree.run(&facts(&[("x", true)])).unwrap(), Run { action: "k".into(), path: vec!["a".into(), "b".into(), "z".into()], comparisons: 2 });
    assert!(compile_tree(&deep, 13).is_err());
    assert!(chart_of(&json!({"start": "a", "steps": {"a": {"kind": "jump"}}})).is_err());
}

#[test]
fn edited_routing_cannot_widen_authority() {
    let swapped = with(|v| v["trees"][ROUTING]["steps"]["approval"]["key"] = json!("build"));
    assert!(swapped.err().unwrap().contains("must keep approval"));
    let skipped = with(|v| v["trees"][ROUTING]["start"] = json!("hold"));
    assert!(skipped.is_err());
    let unknown = with(|v| v["trees"][ROUTING]["steps"]["talk"]["key"] = json!("user_said_yes"));
    assert!(unknown.err().unwrap().contains("Unknown routing condition"));
    let reordered = with(|v| {
        let steps = &mut v["trees"][ROUTING]["steps"];
        steps["start"] = json!({"kind": "start", "next": "stop"});
    });
    assert!(reordered.err().unwrap().contains("unreachable"));
    let renamed = with(|v| {
        v["trees"][ROUTING]["steps"]["go"] = json!({"kind": "start", "next": "stop"});
        v["trees"][ROUTING]["start"] = json!("go");
    });
    assert!(renamed.is_ok());
    let missing = with(|v| {
        v["trees"].as_object_mut().unwrap().remove(ROUTING);
    });
    assert_eq!(missing.err().unwrap(), "Missing brain-routing tree.");
    for bits in 0..32u32 {
        let f = facts(&[("stop", bits & 1 != 0), ("hold", bits & 2 != 0), ("risk", bits & 4 != 0), ("talk", bits & 8 != 0), ("review", bits & 16 != 0)]);
        let run = engine().tree(ROUTING, &f).unwrap().unwrap();
        if let Some(required) = required_action(&f) {
            assert_eq!(run.action, required);
        }
    }
}

#[test]
fn risk_keywords_stay_fixed() {
    let risk_id = seed().keywords.iter().find(|k| k.place == "risk").unwrap().id.clone();
    let off = with(|v| {
        for k in v["keywords"].as_array_mut().unwrap() {
            if k["id"] == json!(risk_id) {
                k["on"] = json!(false);
            }
        }
    });
    assert!(off.err().unwrap().contains("is fixed"));
    let risk_alias = seed().keywords.iter().find(|k| k.place == "risk").unwrap().aliases[0].clone();
    let stolen = with(|v| v["keywords"][0]["aliases"].as_array_mut().unwrap().push(json!(format!("{risk_alias} now"))));
    assert!(stolen.err().unwrap().contains("Risk phrase cannot be reassigned"));
    let moved = with(|v| v["keywords"][0]["match"] = json!("hint"));
    assert!(moved.err().unwrap().contains("needs gate placement"));
    let twice = with(|v| v["keywords"][1]["aliases"].as_array_mut().unwrap().push(json!("discuss")));
    assert!(twice.err().unwrap().contains("Unreachable alias"));
    let odd = with(|v| v["keywords"][0]["effect"] = json!("grant-approval"));
    assert!(odd.err().unwrap().contains("Invalid or duplicate keyword"));
}

#[test]
fn table_and_answer_rules_are_checked() {
    let no_default = with(|v| v["decisions"].as_array_mut().unwrap().retain(|r| !(r["kind"] == json!("general") && r["capability"] == json!("*"))));
    assert!(no_default.err().unwrap().contains("general/*"));
    let bad_tier = with(|v| v["decisions"][0]["tier"] = json!("free"));
    assert_eq!(bad_tier.err().unwrap(), "Invalid decision table row.");
    let dup = with(|v| {
        let first = v["decisions"][0].clone();
        v["decisions"].as_array_mut().unwrap().push(first);
    });
    assert!(dup.err().unwrap().contains("Duplicate decision table key"));
    let unsafe_rule = with(|v| v["answerRules"] = json!([{"id": 1, "project": "nixa", "ask": "may I continue", "answer": "yes, git push origin main", "status": "live"}]));
    assert!(unsafe_rule.err().unwrap().contains("unsafe answer rule 1"));
    let good_rule = with(|v| v["answerRules"] = json!([{"id": 1, "project": "nixa", "ask": "run the tests?", "answer": "yes", "status": "trial"}]));
    assert!(good_rule.is_ok());
    assert!(with(|v| v["maxDepth"] = json!(13)).is_err());
    assert!(with(|v| v["version"] = json!(2)).is_err());
    assert!(rules_of(&json!([])).is_err());
}

#[test]
fn decisions_never_grant_approval() {
    let engine = engine();
    let risky = engine.decide("ok go ahead and push it to main", &Input { held: true, ..Input::default() }).unwrap();
    assert!(risky.requires_approval && !risky.grants_approval);
    assert_eq!(risky.action, "approval");
    let held = engine.decide("look at the logs", &Input { held: true, ..Input::default() }).unwrap();
    assert_eq!(held.action, "hold");
    let freed = engine.decide("go ahead", &Input { held: true, ..Input::default() }).unwrap();
    assert_eq!(freed.action, "build");
    let card = engine.decide("go ahead", &Input { held: true, card_open: true, ..Input::default() }).unwrap();
    assert_eq!(card.action, "hold");
    let denied = engine.decide("don't build it", &Input::default()).unwrap();
    assert!(!denied.keywords.iter().any(|k| k == "build-mode"));
    let table = engine.decide("fix it", &Input { kind: Some("review".into()), capability: Some("ui".into()), ..Input::default() }).unwrap();
    assert_eq!((table.mode.as_str(), table.tier.as_str()), ("review", "premium"));
    let other = engine.lookup("nothing", "odd");
    assert_eq!((other.kind.as_str(), other.capability.as_str()), ("general", "*"));
}

#[test]
fn files_seed_reload_and_keep_last_good() {
    let dir = tempfile::tempdir().unwrap();
    let mut trees = FastTrees::open(dir.path());
    assert!(trees.problem.is_none(), "{:?}", trees.problem);
    let file = dir.path().join(FILE);
    let tree_file = dir.path().join(TREES_DIR).join(format!("{ROUTING}.json"));
    assert!(file.exists() && tree_file.exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(dir.path().join(TREES_DIR)).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(trees.reload(), Reload { changed: false, problem: None });
    let mut chart: Value = serde_json::from_str(&std::fs::read_to_string(&tree_file).unwrap()).unwrap();
    chart["steps"]["approval"]["key"] = json!("build");
    std::fs::write(&tree_file, chart.to_string()).unwrap();
    let bad = trees.reload();
    assert!(!bad.changed && bad.problem.unwrap().starts_with("Fast decisions kept the last good rules: Brain routing must keep approval"));
    assert_eq!(trees.reload(), Reload { changed: false, problem: None });
    assert_eq!(trees.decide("deploy it", &Input::default()).unwrap().action, "approval");
    chart["steps"]["approval"]["key"] = json!("approval");
    chart["steps"]["review"]["label"] = json!("Review now?");
    std::fs::write(&tree_file, chart.to_string()).unwrap();
    assert_eq!(trees.reload(), Reload { changed: true, problem: None });
    std::fs::write(dir.path().join(TREES_DIR).join("Bad Name.json"), "{}").unwrap();
    assert!(trees.reload().problem.unwrap().contains("Invalid decision tree file name"));
    std::fs::remove_file(dir.path().join(TREES_DIR).join("Bad Name.json")).unwrap();
    assert!(trees.reload().changed);
    let saved = trees
        .update(|v| {
            for k in v["keywords"].as_array_mut().unwrap() {
                if k["id"] == json!("discuss") {
                    k["on"] = json!(false);
                }
            }
        })
        .unwrap();
    assert_eq!(saved["trees"][ROUTING]["steps"]["review"]["label"], json!("Review now?"));
    assert!(!trees.decide("discuss it", &Input::default()).unwrap().keywords.contains(&"discuss".to_string()));
    assert_eq!(trees.reload(), Reload { changed: false, problem: None });
    assert!(trees.update(|v| v["trees"][ROUTING]["steps"]["stopped"]["key"] = json!("build")).is_err());
    assert!(!trees.decide("discuss it", &Input::default()).unwrap().keywords.contains(&"discuss".to_string()));
}

#[test]
fn broken_file_falls_back_to_backup_then_seed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    std::fs::write(&file, "{ not json").unwrap();
    let trees = FastTrees::open(dir.path());
    assert!(trees.problem.as_deref().unwrap().starts_with("Fast decisions kept the last good rules: Invalid fast-decisions.json"));
    assert_eq!(trees.decide("stop", &Input::default()).unwrap().action, "stop");
    let mut backup = seed_value();
    for k in backup["keywords"].as_array_mut().unwrap() {
        if k["id"] == json!("discuss") {
            k["on"] = json!(false);
        }
    }
    std::fs::write(dir.path().join(format!("{FILE}.bak")), backup.to_string()).unwrap();
    let trees = FastTrees::open(dir.path());
    assert!(!trees.decide("discuss it", &Input::default()).unwrap().keywords.contains(&"discuss".to_string()));
}

#[test]
fn note_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(dir.path());
    assert!(first_problem(&state).is_none());
    assert!(tick(&state).is_none());
    let note = note(&state, "plan it with the best model", false, true).unwrap();
    assert!(note.starts_with("Note: Fast local decision"));
    assert!(note.contains("\"grantsApproval\":false"));
    assert!(note.ends_with(DEEP_HINT));
    let quiet = super::note(&state, "plan it with the best model", false, false).unwrap();
    assert!(!quiet.contains("nixa_deep_ask"));
}

#[test]
fn automatic_note_has_no_approval_gate() {
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(dir.path());
    let manual = decide(&state, "deploy it to production", &Input::default()).unwrap();
    assert!(manual.requires_approval);
    let auto = automatic_decision(manual);
    assert!(!auto.requires_approval);
    assert_ne!(auto.action, "approval");
    let note = super::note(&state, "deploy it to production", false, false).unwrap();
    assert!(note.contains("\"requiresApproval\":false") && note.contains("There is no approval gate") && !note.contains("risk gates still apply"));
}

#[test]
fn check_report_works_on_a_temp_copy() {
    let dir = tempfile::tempdir().unwrap();
    let built_in = report(&[], dir.path()).unwrap();
    assert_eq!(built_in["source"], json!("built-in rules"));
    assert_eq!(built_in["ok"], json!(true));
    assert_eq!(built_in["sample"], json!(SAMPLE));
    assert_eq!(built_in["decision"]["action"], json!("plan"));
    let _ = FastTrees::open(dir.path());
    let args: Vec<String> = ["--sample", "go ahead", "--held", "--bench"].iter().map(|s| s.to_string()).collect();
    let made = report(&args, dir.path()).unwrap();
    assert_eq!(made["treeFiles"].as_array().unwrap().len(), 1);
    assert_eq!(made["decision"]["action"], json!("build"));
    assert!(made["averageMicroseconds"].as_f64().unwrap() > 0.0);
    let gone: Vec<String> = ["--rules", "/nowhere/x.json"].iter().map(|s| s.to_string()).collect();
    assert!(report(&gone, dir.path()).unwrap_err().contains("No rules file"));
    let empty: Vec<String> = vec!["--sample".into()];
    assert_eq!(report(&empty, dir.path()).unwrap_err(), "--sample needs a value.");
}
