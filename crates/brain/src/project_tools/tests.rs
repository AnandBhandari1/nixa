use super::find::{ProjectSpace, find_project_folder};
use super::policy::{glob_regex, link_chain, nixa_data_inside, project_path_in, protected_in, secret_part, sensitive_file, sensitive_name};
use super::project_of::{NO_FOLDER, project_of, root_from};
use super::search::{matcher, positive, read_range, search_folder};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

fn real(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap()
}

fn git_init(dir: &Path) {
    let ok = std::process::Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir).status().unwrap().success();
    assert!(ok);
}

#[test]
fn secret_parts_are_known() {
    for name in [".env", ".env.local", ".envrc", "prod.env", ".ssh", "id_ed25519", "server.pem", "api.key", ".zsh_history", "secrets.json", "credentials", ".git", ".npmrc"] {
        assert!(secret_part(name), "{name}");
    }
    for name in ["main.rs", "README.md", "environment.ts", "keys.rs", "monkey.txt"] {
        assert!(!secret_part(name), "{name}");
    }
}

#[test]
fn private_roots_and_live_data_are_blocked() {
    let roots = vec![PathBuf::from("/home/x/.claude"), PathBuf::from("/data/nixa")];
    assert!(protected_in(Path::new("/home/x/.claude/settings.json"), &roots));
    assert!(protected_in(Path::new("/data/nixa/nixa.db"), &roots));
    assert!(protected_in(Path::new("/p/app/.config/nixa/nixa.db"), &roots));
    assert!(nixa_data_inside(Path::new("/a/.config/Nixa")));
    assert!(!protected_in(Path::new("/p/app/src/main.rs"), &roots));
}

#[test]
fn sensitive_names_and_links() {
    assert!(glob_regex("*.pem").is_match("a.pem"));
    assert!(!glob_regex("*.pem").is_match("a.pemx"));
    assert!(sensitive_name(Path::new("/x/Credentials.json")));
    assert!(sensitive_name(Path::new("/x/.env:stream")));
    assert!(sensitive_name(Path::new("/x/.env. ")));
    assert!(!sensitive_name(Path::new("/x/notes.txt")));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "A=1").unwrap();
    std::os::unix::fs::symlink(dir.path().join(".env"), dir.path().join("plain")).unwrap();
    assert!(link_chain(&dir.path().join("plain")).len() >= 2);
    assert!(sensitive_file(&dir.path().join("plain")));
}

#[test]
fn project_paths_stay_inside() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
    std::fs::write(root.join(".env"), "KEY=1").unwrap();
    std::fs::write(dir.path().join("outside.txt"), "x").unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside.txt"), root.join("escape")).unwrap();
    std::os::unix::fs::symlink(root.join(".env"), root.join("config.txt")).unwrap();
    let project = root.display().to_string();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let roots: Vec<PathBuf> = Vec::new();
    assert_eq!(project_path_in(&project, "src/a.rs", Some(&home), &roots).unwrap(), real(&root).join("src/a.rs"));
    assert!(project_path_in("proj", ".", Some(&home), &roots).unwrap_err().contains("absolute"));
    assert!(project_path_in("/", ".", Some(&home), &roots).is_err());
    assert!(project_path_in(&dir.path().display().to_string(), ".", Some(&home), &roots).unwrap_err().contains("home folder"));
    assert!(project_path_in(&project, "../outside.txt", Some(&home), &roots).unwrap_err().contains("outside"));
    assert!(project_path_in(&project, "escape", Some(&home), &roots).unwrap_err().contains("outside"));
    assert!(project_path_in(&project, ".env", Some(&home), &roots).unwrap_err().contains("Secret"));
    assert!(project_path_in(&project, "config.txt", Some(&home), &roots).unwrap_err().contains("Secret"));
}

#[test]
fn search_finds_and_skips() {
    let dir = tempfile::tempdir().unwrap();
    let root = real(dir.path());
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::write(root.join("src/a.rs"), "one\nTwo needle\nthree needle\n").unwrap();
    std::fs::write(root.join("b.txt"), "needle here\r\n").unwrap();
    std::fs::write(root.join("node_modules/x/c.js"), "needle").unwrap();
    std::fs::write(root.join(".hidden/d.txt"), "needle").unwrap();
    std::fs::write(root.join(".env"), "needle=1").unwrap();
    std::fs::write(root.join("e.pem"), "needle").unwrap();
    std::fs::write(root.join("bin.dat"), b"needle\0\x01").unwrap();
    let pattern = matcher("NEEDLE", false, false).unwrap();
    let found = search_folder(&root, &root, &pattern, 50, Duration::from_secs(5));
    let hits: Vec<(String, usize)> = found.matches.iter().map(|m| (m.path.clone(), m.line)).collect();
    assert_eq!(hits, vec![("b.txt".to_string(), 1), ("src/a.rs".to_string(), 2), ("src/a.rs".to_string(), 3)]);
    assert_eq!(found.matches[0].text, "needle here");
    assert!(!found.more);
    let few = search_folder(&root, &root, &pattern, 1, Duration::from_secs(5));
    assert!(few.more);
    assert_eq!(few.matches.len(), 1);
    let one = search_folder(&root, &root.join("src/a.rs"), &matcher("three", false, true).unwrap(), 10, Duration::from_secs(5));
    assert_eq!(one.matches.len(), 1);
    assert!(search_folder(&root, &root, &matcher("needle", false, true).unwrap(), 50, Duration::ZERO).stopped.is_some());
    assert!(matcher("(", true, false).unwrap_err().contains("regex"));
    assert!(matcher("a.c", false, false).unwrap().is_match("a.c"));
    assert!(!matcher("a.c", false, false).unwrap().is_match("abc"));
    assert!(matcher("  ", false, false).is_err());
}

#[test]
fn read_lines_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "l1\nl2\nl3\nl4\n").unwrap();
    let range = read_range(&file, 2, 2).unwrap();
    assert_eq!(range.lines, vec!["2: l2", "3: l3"]);
    assert_eq!((range.from, range.to, range.end), (2, 3, false));
    let rest = read_range(&file, 3, 10).unwrap();
    assert!(rest.end);
    assert_eq!(rest.to, 4);
    let past = read_range(&file, 9, 10).unwrap();
    assert!(past.lines.is_empty() && past.end && past.to == 8);
    std::fs::write(dir.path().join("b.bin"), b"a\0b").unwrap();
    assert!(read_range(&dir.path().join("b.bin"), 1, 5).is_err());
    assert_eq!(positive(Some(&json!(500)), 200, 400), 400);
    assert_eq!(positive(Some(&json!(0)), 200, 400), 200);
    assert_eq!(positive(Some(&json!("7")), 200, 400), 7);
    assert_eq!(positive(None, 200, 400), 200);
}

#[test]
fn finds_projects_by_space_folder_and_repo() {
    let dir = tempfile::tempdir().unwrap();
    let projects = real(dir.path()).join("Projects");
    let app = projects.join("group").join("app");
    std::fs::create_dir_all(app.join(".git")).unwrap();
    let web = projects.join("web");
    std::fs::create_dir_all(&web).unwrap();
    let spaces = vec![ProjectSpace { id: "s1".into(), name: "Website".into(), folder: web.display().to_string(), ..ProjectSpace::default() }];
    assert_eq!(find_project_folder("website", &spaces, &projects).unwrap(), web.display().to_string());
    assert_eq!(find_project_folder("web", &spaces, &projects).unwrap(), web.display().to_string());
    assert_eq!(find_project_folder("app", &spaces, &projects).unwrap(), app.display().to_string());
    assert_eq!(find_project_folder(&app.display().to_string(), &spaces, &projects).unwrap(), app.display().to_string());
    let missing = find_project_folder("nothing", &spaces, &projects).unwrap_err();
    assert!(missing.contains("Website"), "{missing}");
    assert!(find_project_folder("a/../b", &spaces, &projects).is_err());
    assert!(find_project_folder(" ", &spaces, &projects).is_err());
}

#[tokio::test]
async fn project_of_finds_the_repo_root() {
    assert_eq!(root_from("/a/b/.git", ""), Some("/a/b".to_string()));
    assert_eq!(root_from("/a/b/.git/worktrees/x", "/w"), Some("/w".to_string()));
    assert_eq!(root_from("", ""), None);
    let dir = tempfile::tempdir().unwrap();
    let root = real(dir.path());
    git_init(&root);
    std::fs::create_dir_all(root.join("deep/er")).unwrap();
    let known = Mutex::new(Default::default());
    let folder = root.join("deep/er").display().to_string();
    assert_eq!(project_of(&known, Some(&folder)).await, root.display().to_string());
    assert_eq!(crate::app::lock(&known).len(), 1);
    assert_eq!(project_of(&known, None).await, NO_FOLDER);
    let plain = tempfile::tempdir().unwrap();
    let alone = plain.path().display().to_string();
    assert_eq!(project_of(&known, Some(&alone)).await, alone);
}

#[tokio::test]
async fn git_inspect_leaves_secrets_out() {
    let dir = tempfile::tempdir().unwrap();
    let root = real(dir.path());
    git_init(&root);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    std::fs::write(root.join(".env"), "KEY=old\n").unwrap();
    let add = std::process::Command::new("git").args(["add", "-f", "a.txt", ".env"]).current_dir(&root).status().unwrap();
    assert!(add.success());
    std::fs::write(root.join("a.txt"), "two\n").unwrap();
    let project = root.display().to_string();
    let staged = super::files::inspect_git(&project, "staged").await.unwrap();
    assert_eq!(staged["omitted_paths"], 1);
    let output = staged["output"].as_str().unwrap();
    assert!(output.contains("a.txt") && !output.contains("KEY"));
    let status = super::files::inspect_git(&project, "status").await.unwrap();
    assert!(status["output"].as_str().unwrap().contains("a.txt"));
    assert!(super::files::inspect_git(&project, "push").await.is_err());
    let listed = super::files::list_dir(&project, ".").unwrap();
    let names: Vec<&str> = listed["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["a.txt"]);
    assert_eq!(super::files::read_file(&project, "a.txt").unwrap()["text"], "two\n");
    assert!(super::files::read_file(&project, ".env").is_err());
}

#[test]
fn credential_folders_and_linked_home_dirs_are_refused() {
    use super::policy::{SECRET_REFUSED, private_roots};
    let dir = tempfile::tempdir().unwrap();
    let home = real(dir.path()).join("home");
    let data = home.join("Library/Application Support/Nixa");
    std::fs::create_dir_all(&data).unwrap();
    let roots = private_roots(Some(&home), &data);
    let opencode = home.join(".local/share/opencode");
    std::fs::create_dir_all(&opencode).unwrap();
    std::fs::write(opencode.join("auth.json"), "{}").unwrap();
    let local = project_path_in(&opencode.display().to_string(), "auth.json", Some(&home), &roots).unwrap_err();
    assert_eq!(local, SECRET_REFUSED);
    let dotfiles = home.join("dotfiles/codex");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::write(dotfiles.join("config.toml"), "x").unwrap();
    std::os::unix::fs::symlink(&dotfiles, home.join(".codex")).unwrap();
    let roots = private_roots(Some(&home), &data);
    let linked = project_path_in(&home.join(".codex").display().to_string(), "config.toml", Some(&home), &roots).unwrap_err();
    assert_eq!(linked, SECRET_REFUSED);
    let dev = home.join("Library/Application Support/Nixa Dev");
    assert!(protected_in(&dev.join("window-token"), &roots));
    assert!(protected_in(&home.join("Library/Cookies/x"), &roots));
    assert!(!protected_in(&home.join("Library/CloudStorage/Dropbox/notes.md"), &roots));
    for name in ["auth.json", "hosts.yml", "logins.json", "accessTokens.json", "msal_token_cache.json", "Local State"] {
        assert!(sensitive_name(Path::new(name)), "{name}");
    }
}

#[test]
fn an_absolute_folder_must_be_a_space_a_project_or_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    let plain = real(dir.path()).join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let projects = real(dir.path()).join("Projects");
    let refused = find_project_folder(&plain.display().to_string(), &[], &projects).unwrap_err();
    assert!(refused.contains("not a git repo"), "{refused}");
    let spaces = vec![ProjectSpace { folder: plain.display().to_string(), ..ProjectSpace::default() }];
    assert_eq!(find_project_folder(&plain.display().to_string(), &spaces, &projects).unwrap(), plain.display().to_string());
}
