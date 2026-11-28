use super::gate::{GATE_NAME, GateEnv, LOOP_CODE, MAX_DEPTH, MISSING_CODE, Plan, find_real_gh, gate_dir, gate_file, gate_ready, gate_script, gated_line, is_gate_script, path_with_gate, plan, run_plan, write_gate};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const TOOLS: &str = "/usr/bin:/bin";

fn s(list: &[&str]) -> Vec<String> {
    list.iter().map(|x| x.to_string()).collect()
}

fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Place {
    _dir: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    real: PathBuf,
    nixa: PathBuf,
    home: PathBuf,
}

fn place() -> Place {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let data = root.join("data");
    let real = root.join("realbin");
    let nixa = root.join("nixabin").join("nixa");
    let home = root.join("home");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    script(&real.join("gh"), "echo \"real $*\"\nexit 7");
    script(&nixa, "echo \"gate $*\"");
    for (name, text) in [(".zshrc", "export A=1\n"), (".zprofile", "export B=1\n"), (".gitconfig", "[user]\n")] {
        std::fs::write(home.join(name), text).unwrap();
    }
    Place { _dir: dir, root, data, real, nixa, home }
}

fn run_sh(path: &str, home: &Path, line: &str) -> String {
    let out = Command::new("/bin/sh").args(["-c", line]).env_clear().env("PATH", path).env("HOME", home).output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string()
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                todo.push(path);
            } else {
                out.insert(path.display().to_string(), std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    out
}

fn env_of(p: &Place, args: &[&str], path: &str) -> GateEnv {
    GateEnv { args: s(args), path: Some(path.into()), own: vec![p.nixa.clone()], gate_dir: Some(gate_dir(&p.data)), now: 1000, ..GateEnv::default() }
}

#[test]
fn gate_file_is_made_only_in_the_data_folder() {
    let p = place();
    let before = tree(&p.root);
    let file = write_gate(&p.data, &p.nixa).unwrap();
    assert_eq!(file, gate_file(&p.data));
    assert_eq!(file, p.data.join("git").join("bin").join(GATE_NAME));
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(gate_dir(&p.data)).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(is_gate_script(&file));
    assert!(gate_ready(&p.data));
    let after = tree(&p.root);
    let added: Vec<&String> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    assert_eq!(added, vec![&file.display().to_string()]);
    for (name, bytes) in &before {
        assert_eq!(after.get(name), Some(bytes), "{name} changed");
    }
    let made = std::fs::metadata(&file).unwrap().modified().unwrap();
    write_gate(&p.data, &p.nixa).unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), made);
    assert!(gate_script(Path::new("nixa"), &gate_dir(&p.data)).is_err());
    assert!(gate_script(Path::new("/a\nb"), &gate_dir(&p.data)).is_err());
    let odd = gate_script(Path::new("/x/it's/nixa"), &gate_dir(&p.data)).unwrap();
    assert!(odd.contains(r"'/x/it'\''s/nixa'"));
    assert!(!odd.contains("@MAX@") && !odd.contains("@BIN@") && !odd.contains("@DIR@") && !odd.contains("@MARK@") && !odd.contains("@LOOP@"));
}

#[test]
fn only_gated_terminals_find_the_gate() {
    let p = place();
    write_gate(&p.data, &p.nixa).unwrap();
    let plain = format!("{}:{TOOLS}", p.real.display());
    let (gated, note) = path_with_gate(&p.data, &plain);
    assert!(note.is_none());
    assert!(gated.starts_with(&gate_dir(&p.data).display().to_string()));
    assert_eq!(path_with_gate(&p.data, &gated).0, gated);
    assert_eq!(run_sh(&gated, &p.home, "command -v gh"), gate_file(&p.data).display().to_string());
    assert_eq!(run_sh(&gated, &p.home, "gh pr list"), "gate gh pr list");
    assert_eq!(run_sh(&plain, &p.home, "command -v gh"), p.real.join("gh").display().to_string());
    assert_eq!(run_sh(&plain, &p.home, "gh pr list"), "real pr list");
}

#[test]
fn login_zsh_profile_can_win_and_the_start_line_fixes_it() {
    let p = place();
    write_gate(&p.data, &p.nixa).unwrap();
    let brew = p.root.join("brew");
    script(&brew.join("gh"), "echo brew");
    let zdot = p.root.join("zdot");
    std::fs::create_dir_all(&zdot).unwrap();
    std::fs::write(zdot.join(".zprofile"), format!("export PATH=\"{}:$PATH\"\n", brew.display())).unwrap();
    let (gated, _) = path_with_gate(&p.data, &format!("{}:{TOOLS}", p.real.display()));
    let zsh = |line: &str| {
        let out = Command::new("/bin/zsh").args(["-l", "-c", line]).env_clear().env("PATH", &gated).env("HOME", &p.home).env("ZDOTDIR", &zdot).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(zsh("command -v gh"), brew.join("gh").display().to_string());
    assert_eq!(zsh(&gated_line(&p.data, "command -v gh")), gate_file(&p.data).display().to_string());
    assert_eq!(zsh(&gated_line(&p.data, "gh pr view 1")), "gate gh pr view 1");
}

#[test]
fn login_sh_start_line_keeps_the_gate_first() {
    let p = place();
    write_gate(&p.data, &p.nixa).unwrap();
    let brew = p.root.join("brew");
    script(&brew.join("gh"), "echo brew");
    let home = p.root.join("shhome");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".profile"), format!("export PATH=\"{}:$PATH\"\n", brew.display())).unwrap();
    let (gated, _) = path_with_gate(&p.data, &format!("{}:{TOOLS}", p.real.display()));
    let sh = |line: &str| {
        let out = Command::new("/bin/sh").args(["-lc", line]).env_clear().env("PATH", &gated).env("HOME", &home).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let order = sh("printf '%s' \"$PATH\"");
    let dir = gate_dir(&p.data).display().to_string();
    let at = |part: &str| order.split(':').position(|d| d == part);
    assert!(at(&dir) > at("/usr/bin"), "{order}");
    assert_eq!(sh(&gated_line(&p.data, "command -v gh")), gate_file(&p.data).display().to_string());
    assert_eq!(sh(&gated_line(&p.data, "gh pr view 2")), "gate gh pr view 2");
}

#[test]
fn bypasses_are_known_limits() {
    let p = place();
    write_gate(&p.data, &p.nixa).unwrap();
    let (gated, _) = path_with_gate(&p.data, &format!("{}:{TOOLS}", p.real.display()));
    let gate = gate_file(&p.data).display().to_string();
    assert_eq!(run_sh(&gated, &p.home, &format!("{} pr merge 1", p.real.join("gh").display())), "real pr merge 1");
    assert_eq!(run_sh(&gated, &p.home, "command -v gh"), gate);
    assert_ne!(run_sh(&gated, &p.home, "env -i /bin/sh -c 'command -v gh'"), gate);
    assert_eq!(run_sh(&gated, &p.home, &format!("PATH={} command -v gh", p.real.display())), p.real.join("gh").display().to_string());
}

#[test]
fn real_gh_skips_the_gate_and_its_copies() {
    let p = place();
    write_gate(&p.data, &p.nixa).unwrap();
    let dir = gate_dir(&p.data);
    let copy = p.root.join("copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(gate_file(&p.data), copy.join("gh")).unwrap();
    let link = p.root.join("link");
    std::fs::create_dir_all(&link).unwrap();
    std::fs::hard_link(&p.nixa, link.join("gh")).unwrap();
    let alias = p.root.join("alias");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    let path = format!("{}:{}:{}:{}:{}", dir.display(), alias.display(), copy.display(), link.display(), p.real.display());
    assert_eq!(find_real_gh(&env_of(&p, &["pr"], &path)).unwrap(), p.real.join("gh"));
    let only_gate = format!("{}:{}", dir.display(), copy.display());
    assert!(find_real_gh(&env_of(&p, &["pr"], &only_gate)).unwrap_err().contains("not found"));
    let mut given = env_of(&p, &["pr"], &path);
    given.real_gh = Some(gate_file(&p.data).display().to_string());
    assert!(find_real_gh(&given).unwrap_err().contains("NIXA_REAL_GH"));
    given.real_gh = Some("gh".into());
    assert!(find_real_gh(&given).is_err());
    given.real_gh = Some(link.join("gh").display().to_string());
    assert!(find_real_gh(&given).is_err());
    given.real_gh = Some(p.real.join("gh").display().to_string());
    assert_eq!(find_real_gh(&given).unwrap(), p.real.join("gh"));
}

#[test]
fn loop_guard_and_failures() {
    let p = place();
    let path = p.real.display().to_string();
    let mut env = env_of(&p, &["pr", "list"], &path);
    env.depth = Some(MAX_DEPTH.to_string());
    let Plan::Refuse(code, text) = plan(&env) else { panic!("loop not stopped") };
    assert_eq!(code, LOOP_CODE);
    assert!(text.contains("called itself"));
    env.depth = Some("1".into());
    assert_eq!(plan(&env), Plan::Run(p.real.join("gh")));
    env.depth = Some("junk".into());
    assert_eq!(plan(&env), Plan::Run(p.real.join("gh")));
    let mut write = env_of(&p, &["pr", "merge", "1"], &path);
    write.chain = s(&["codex exec"]);
    write.terminal = Some("m2".into());
    assert_eq!(plan(&write), Plan::Run(p.real.join("gh")));
    assert_eq!(run_plan(&write).0, 7);
    let mut read = write.clone();
    read.args = s(&["pr", "view", "1"]);
    assert_eq!(run_plan(&read).0, 7);
    let started = Instant::now();
    let none = env_of(&p, &["pr", "list"], &p.root.join("empty").display().to_string());
    let (code, text) = run_plan(&none);
    assert_eq!(code, MISSING_CODE);
    assert!(text.contains("not found"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn depth_is_passed_to_the_real_gh() {
    let p = place();
    let seen = p.root.join("seen.txt");
    script(&p.real.join("gh"), &format!("echo \"$NIXA_GH_GATE_DEPTH $*\" > {}\nexit 4", seen.display()));
    let (code, _) = run_plan(&env_of(&p, &["repo", "view"], &p.real.display().to_string()));
    assert_eq!(code, 4);
    assert_eq!(std::fs::read_to_string(&seen).unwrap().trim(), "1 repo view");
}

#[test]
fn missing_gate_falls_back_to_the_normal_gh() {
    let p = place();
    let plain = format!("{}:{TOOLS}", p.real.display());
    let (same, note) = path_with_gate(&p.data, &plain);
    assert_eq!(same, plain);
    assert!(note.unwrap().contains("normal gh"));
    assert_eq!(gated_line(&p.data, "claude"), "claude");
    write_gate(&p.data, &p.nixa).unwrap();
    std::fs::set_permissions(gate_file(&p.data), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!gate_ready(&p.data));
    assert_eq!(path_with_gate(&p.data, &plain).0, plain);
    std::fs::set_permissions(gate_file(&p.data), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(gate_dir(&p.data), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!gate_ready(&p.data));
    std::fs::set_permissions(gate_dir(&p.data), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::remove_file(gate_file(&p.data)).unwrap();
    let gone_nixa = p.root.join("nowhere").join("nixa");
    write_gate(&p.data, &gone_nixa).unwrap();
    let (gated, _) = path_with_gate(&p.data, &plain);
    let out = run_sh(&gated, &p.home, "gh pr list");
    assert!(out.contains("real pr list"), "{out}");
    assert!(out.contains("nixa command is missing"), "{out}");
    let looped = run_sh(&gated, &p.home, &format!("NIXA_GH_GATE_DEPTH={MAX_DEPTH} gh pr list; echo code=$?"));
    assert!(looped.contains("called itself") && looped.contains(&format!("code={LOOP_CODE}")), "{looped}");
    let none = run_sh(&format!("{}:{TOOLS}", gate_dir(&p.data).display()), &p.home, "gh pr list; echo code=$?");
    assert!(none.contains("nixa command is missing") && none.contains("code=127"), "{none}");
}

#[test]
fn every_gate_folder_in_the_data_folder_is_private() {
    let p = place();
    std::fs::set_permissions(&p.data, std::fs::Permissions::from_mode(0o750)).unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    write_gate(&p.data, &p.nixa).unwrap();
    assert_eq!(mode(&p.data.join("git")), 0o700);
    assert_eq!(mode(&gate_dir(&p.data)), 0o700);
    assert_eq!(mode(&gate_file(&p.data)), 0o700);
    assert_eq!(mode(&p.data), 0o750);
    std::fs::set_permissions(p.data.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    write_gate(&p.data, &p.nixa).unwrap();
    assert_eq!(mode(&p.data.join("git")), 0o700);
}

#[test]
fn github_writes_go_in_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("work").display().to_string();
    super::gate::note_auto(dir.path(), "pr create", "codex", Some("m2"), Some(folder.as_str()));
    let conn = nixa_store::open(&dir.path().join(nixa_store::DB_FILE)).unwrap();
    let rows = crate::modes::journal::last_for(&conn, None, 5).unwrap();
    assert!(rows.iter().any(|row| row.choice.contains("gh pr create")), "{rows:?}");
}
