use super::mermaid::{drone_mmd, parse, tree_mmd};
use super::*;
use crate::drones::default_drone_tree;
use crate::trees::{check_tree, default_finished_tree};

fn finished() -> RuleTree {
    check_tree(&default_finished_tree(), None).unwrap()
}

#[test]
fn a_tree_goes_to_mermaid_and_back_unchanged() {
    let tree = finished();
    let parsed = parse(&tree_mmd(&tree)).unwrap();
    let (back, notes) = apply_tree(tree.clone(), &parsed).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(back, tree);
}

#[test]
fn flowman_edits_change_tree_text_boxes_and_arrows() {
    let tree = finished();
    let start = tree.start.clone();
    let mmd = tree_mmd(&tree).replace(&tree.boxes[&start].text.replace('"', "#quot;"), "Is the work a job?");
    let extra = format!("{mmd}    n_new1[\"Ask the deep brain\"]\n    n_{start} -->|\"Maybe\"| n_new1\n");
    let (back, notes) = apply_tree(tree, &parse(&extra).unwrap()).unwrap();
    assert_eq!(back.boxes[&start].text, "Is the work a job?");
    assert_eq!(back.boxes["new1"].text, "Ask the deep brain");
    assert_eq!(back.boxes[&start].next.as_ref().unwrap()["Maybe"], "new1");
    assert!(notes.iter().any(|n| n.contains("added box new1")), "{notes:?}");
    assert!(check_tree(&serde_json::to_value(&back).unwrap(), None).is_ok());
}

#[test]
fn an_arrow_without_a_word_is_refused() {
    let tree = finished();
    let start = tree.start.clone();
    let mmd = format!("{}    n_{start} --> n_{start}\n", tree_mmd(&tree));
    assert!(apply_tree(tree, &parse(&mmd).unwrap()).unwrap_err().contains("no word"));
}

#[test]
fn drone_ids_with_dashes_go_back_and_forth_and_only_text_changes() {
    let tree = default_drone_tree();
    let mmd = drone_mmd(&tree);
    assert!(mmd.contains("n_a__agent"), "{mmd}");
    let parsed = parse(&mmd).unwrap();
    assert!(parsed.nodes.contains_key("a-agent"));
    let (_, changed) = apply_drones(tree.clone(), &parsed).unwrap();
    assert!(changed.is_empty(), "{changed:?}");
    let label = mermaid::drone_text(&tree.steps["a-agent"]).replace('"', "#quot;");
    let edited = parse(&mmd.replacen(&label, "Agent state now", 1)).unwrap();
    let (_, changed) = apply_drones(tree.clone(), &edited).unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].1.label(), Some("Agent state now"));
    let moved = parse(&format!("{mmd}    n_a__agent -->|\"x\"| n_a__agent\n")).unwrap();
    assert!(apply_drones(tree, &moved).unwrap_err().contains("box text only"));
}

#[test]
fn flow_names_parse() {
    assert_eq!(Flow::parse("drones").unwrap(), Flow::Drones);
    assert_eq!(Flow::parse("tree/agent-finished").unwrap().file_name(), "tree-agent-finished.mmd");
    assert_eq!(Flow::parse("workflow/ship").unwrap().key(), "workflow/ship");
    assert!(Flow::parse("other").is_err());
}

#[test]
fn flowman_run_takes_only_mmd_files_in_allowed_folders() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let space = root.join("space");
    std::fs::create_dir_all(space.join("docs")).unwrap();
    std::fs::create_dir_all(root.join("home/.codex")).unwrap();
    std::fs::write(root.join("home/.codex/auth.json"), "{}").unwrap();
    let dirs = vec![space.clone()];
    let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let auth = root.join("home/.codex/auth.json").display().to_string();
    assert!(check_run_words(&words(&["read", &auth]), &dirs).is_err());
    let outside = root.join("home/a.mmd").display().to_string();
    assert!(check_run_words(&words(&["validate", &outside]), &dirs).is_err());
    let good = space.join("docs/01-a.mmd").display().to_string();
    assert!(check_run_words(&words(&["write", &good, "--from", "-"]), &dirs).is_ok());
    assert!(check_run_words(&words(&["write", &good, "--from", &auth]), &dirs).is_err());
    let up = format!("{}/docs/../../home/a.mmd", space.display());
    assert!(check_run_words(&words(&["read", &up]), &dirs).is_err());
    std::os::unix::fs::symlink(&auth, space.join("docs/link.mmd")).unwrap();
    let link = space.join("docs/link.mmd").display().to_string();
    assert!(check_run_words(&words(&["read", &link]), &dirs).is_err());
    let out = space.join("docs/a.svg").display().to_string();
    assert!(check_run_words(&words(&["render", &good, "--out", &out]), &dirs).is_ok());
    assert!(check_run_words(&words(&["status"]), &dirs).is_ok());
    assert!(check_run_words(&words(&["list"]), &dirs).is_err());
    std::os::unix::fs::symlink(root.join("home/gone.mmd"), space.join("docs/dangling.mmd")).unwrap();
    let dangling = space.join("docs/dangling.mmd").display().to_string();
    assert!(check_run_words(&words(&["write", &dangling, "--from", "-"]), &dirs).is_err());
}
