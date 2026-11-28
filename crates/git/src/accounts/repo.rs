use super::Env;
use super::helper::git_str;
use super::profiles::origin_scope;
use super::run::{self, Opts, Output};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const SIGN_FORMATS: &[&str] = &["openpgp", "ssh", "x509"];
pub const LOCK_MIN_AGE: u64 = 60;
pub const GITHUB_KEYS_URL: &str = "https://github.com/settings/keys";

const SIGN_KEYS: &[&str] = &[
    "commit.gpgsign",
    "gpg.format",
    "user.signingkey",
    "gpg.program",
    "gpg.openpgp.program",
    "gpg.ssh.program",
    "gpg.x509.program",
    "gpg.ssh.defaultkeycommand",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub ok: bool,
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
}

impl Done {
    pub fn text(&self) -> String {
        sentence(&[&self.title, &self.hint, &self.detail])
    }
}

pub fn sentence(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            if p.ends_with(['.', '!', '?', ':', '…']) { p.to_string() } else { format!("{p}.") }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn success(title: &str, hint: &str) -> Done {
    Done {
        ok: true,
        title: title.into(),
        hint: hint.into(),
        ..Done::default()
    }
}

fn refusal(kind: &str, title: &str, hint: &str) -> Done {
    Done {
        ok: false,
        kind: kind.into(),
        title: title.into(),
        hint: hint.into(),
        detail: String::new(),
    }
}

fn failure(text: &str, fallback: &str) -> Done {
    let text = if text.trim().is_empty() { fallback } else { text };
    let said = run::describe(text);
    Done {
        ok: false,
        kind: said.kind,
        title: said.title,
        hint: said.hint,
        detail: said.detail,
    }
}

pub fn git_bool(value: Option<&str>) -> Option<bool> {
    let Some(v) = value else {
        return Some(true);
    };
    let v = v.trim().to_lowercase();
    match v.as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" | "" => Some(false),
        _ => v.parse::<i64>().ok().map(|n| n != 0),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub scope: String,
    pub key: String,
    pub value: Option<String>,
}

fn scope_word(scope: &str) -> String {
    match scope {
        "local" | "worktree" => "local".into(),
        other => other.into(),
    }
}

pub fn parse_scoped_config(text: &str, by_origin: bool) -> Vec<Entry> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < parts.len() {
        let entry = parts[i + 1];
        let (key, value) = match entry.find('\n') {
            Some(cut) => (&entry[..cut], Some(entry[cut + 1..].to_string())),
            None => (entry, None),
        };
        let key = key.trim().to_lowercase();
        if !key.is_empty() {
            let scope = if by_origin { origin_scope(parts[i]).to_string() } else { scope_word(parts[i]) };
            out.push(Entry { scope, key, value });
        }
        i += 2;
    }
    out
}

fn last_of<'a>(entries: &'a [Entry], keys: &[&str]) -> Option<&'a Entry> {
    entries.iter().rev().find(|e| keys.contains(&e.key.as_str()))
}

pub fn literal_ssh_key(key: &str) -> bool {
    let k = key.trim();
    ["key::", "ssh-", "ecdsa-", "sk-"].iter().any(|p| k.starts_with(p))
}

pub fn sign_program(format: &str) -> &'static str {
    match format {
        "ssh" => "ssh-keygen",
        "x509" => "gpgsm",
        _ => "gpg",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signing {
    pub enabled: bool,
    pub scope: String,
    pub bad_value: Option<String>,
    pub local_enabled: Option<bool>,
    pub format: String,
    pub format_scope: String,
    pub unknown_format: Option<String>,
    pub key: Option<String>,
    pub key_scope: String,
    pub literal_key: bool,
    pub key_command: Option<String>,
    pub program: String,
    pub program_set: bool,
    pub program_found: Option<bool>,
    pub key_found: Option<bool>,
}

fn value_of(entry: Option<&Entry>) -> String {
    entry.and_then(|e| e.value.as_deref()).unwrap_or("").trim().to_string()
}

pub fn build_signing(entries: &[Entry]) -> Signing {
    let sign = last_of(entries, &["commit.gpgsign"]);
    let locals: Vec<Entry> = entries.iter().filter(|e| e.scope == "local").cloned().collect();
    let local_sign = last_of(&locals, &["commit.gpgsign"]);
    let parsed = sign.map(|s| git_bool(s.value.as_deref())).unwrap_or(Some(false));
    let format_entry = last_of(entries, &["gpg.format"]);
    let raw_format = value_of(format_entry);
    let known = SIGN_FORMATS.contains(&raw_format.as_str());
    let format = if known { raw_format.clone() } else { "openpgp".into() };
    let key_entry = last_of(entries, &["user.signingkey"]);
    let key = Some(value_of(key_entry)).filter(|k| !k.is_empty());
    let program_key = format!("gpg.{format}.program");
    let program_keys: Vec<&str> = if format == "openpgp" {
        vec!["gpg.program", "gpg.openpgp.program"]
    } else {
        vec![program_key.as_str()]
    };
    let program = value_of(last_of(entries, &program_keys));
    let key_command = value_of(last_of(entries, &["gpg.ssh.defaultkeycommand"]));
    Signing {
        enabled: parsed == Some(true),
        scope: sign.map(|s| s.scope.clone()).unwrap_or("none".into()),
        bad_value: sign.filter(|_| parsed.is_none()).map(|s| s.value.clone().unwrap_or_default()),
        local_enabled: local_sign.map(|s| git_bool(s.value.as_deref()) == Some(true)),
        format_scope: match format_entry {
            Some(e) if !raw_format.is_empty() => e.scope.clone(),
            _ => "none".into(),
        },
        unknown_format: (!raw_format.is_empty() && !known).then(|| raw_format.clone()),
        key_scope: match (&key, key_entry) {
            (Some(_), Some(e)) => e.scope.clone(),
            _ => "none".into(),
        },
        literal_key: format == "ssh" && key.as_deref().is_some_and(literal_ssh_key),
        key_command: (format == "ssh" && !key_command.is_empty()).then_some(key_command),
        program_set: !program.is_empty(),
        program: if program.is_empty() { sign_program(&format).to_string() } else { program },
        program_found: None,
        key_found: None,
        key,
        format,
    }
}

const CHECK_SCRIPT: &str = r#"p=$1
k=$2
if command -v "$p" >/dev/null 2>&1; then echo program=1; else echo program=0; fi
if [ -n "$k" ]; then
  case $k in "~/"*) k="$HOME/${k#??}";; esac
  if [ -e "$k" ]; then echo key=1; else echo key=0; fi
fi
exit 0"#;

pub fn flags(text: &str) -> BTreeMap<String, String> {
    run::lines(text)
        .into_iter()
        .map(|l| match l.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (l, String::new()),
        })
        .collect()
}

fn sh(env: &Env, script: &str, args: &[&str], cwd: Option<&Path>) -> Output {
    run::shell(
        script,
        args,
        &Opts {
            cwd: cwd.map(Path::to_path_buf),
            env: vec![("HOME".into(), env.home.display().to_string())],
            timeout: Some(Duration::from_secs(30)),
            ..Opts::default()
        },
    )
}

fn sign_pattern() -> String {
    let keys: Vec<String> = SIGN_KEYS.iter().map(|k| k.replace('.', "\\.")).collect();
    format!("^({})$", keys.join("|"))
}

fn read_signing_entries(env: &Env, root: &Path) -> Result<Vec<Entry>, String> {
    let pattern = sign_pattern();
    let scoped = git_str(env, root, &["config", "--show-scope", "-z", "--get-regexp", &pattern]);
    if scoped.ok() {
        return Ok(parse_scoped_config(&scoped.stdout, false));
    }
    if scoped.code == Some(1) && scoped.stderr.trim().is_empty() {
        return Ok(Vec::new());
    }
    let odd = Regex::new(r"(?i)unknown option|usage:").expect("usage pattern");
    if !odd.is_match(&scoped.stderr) {
        let why = run::tail(&scoped.stderr, 4);
        return Err(if why.is_empty() { "cannot read the git config".into() } else { why });
    }
    let origin = git_str(env, root, &["config", "--show-origin", "-z", "--get-regexp", &pattern]);
    if origin.code == Some(1) && origin.stderr.trim().is_empty() {
        return Ok(Vec::new());
    }
    if !origin.ok() {
        let why = run::tail(&origin.stderr, 4);
        return Err(if why.is_empty() { "cannot read the git config".into() } else { why });
    }
    Ok(parse_scoped_config(&origin.stdout, true))
}

pub fn signing_info(env: &Env, root: &Path) -> Result<Signing, String> {
    let mut info = build_signing(&read_signing_entries(env, root)?);
    let key_path = match (&info.key, info.format.as_str(), info.literal_key) {
        (Some(k), "ssh", false) => k.clone(),
        _ => String::new(),
    };
    let check = sh(env, CHECK_SCRIPT, &[&info.program, &key_path], Some(root));
    let found = if check.ok() { flags(&check.stdout) } else { BTreeMap::new() };
    info.program_found = found.get("program").map(|v| v == "1");
    info.key_found = if info.literal_key { Some(true) } else { found.get("key").map(|v| v == "1") };
    Ok(info)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignChange {
    pub format: Option<Option<String>>,
    pub key: Option<Option<String>>,
    pub enabled: Option<Option<bool>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub args: Vec<String>,
    pub ok_codes: &'static [i32],
}

fn set_step(key: &str, value: &str) -> Step {
    Step {
        args: run::strings(&["config", "--local", key, value]),
        ok_codes: &[0],
    }
}

fn unset_step(key: &str) -> Step {
    Step {
        args: run::strings(&["config", "--local", "--unset-all", key]),
        ok_codes: &[0, 5],
    }
}

pub fn signing_steps(change: &SignChange) -> Result<Vec<Step>, &'static str> {
    let mut steps = Vec::new();
    if let Some(format) = &change.format {
        match format.as_deref() {
            None | Some("") => steps.push(unset_step("gpg.format")),
            Some(f) if SIGN_FORMATS.contains(&f) => steps.push(set_step("gpg.format", f)),
            Some(_) => return Err("Pick SSH, GPG or X.509."),
        }
    }
    if let Some(key) = &change.key {
        let key = key.as_deref().unwrap_or("").trim().to_string();
        if key.contains(['\0', '\r', '\n']) {
            return Err("The key must be on one line.");
        }
        if key.len() > 8192 {
            return Err("The key is too long.");
        }
        if key.starts_with('-') {
            return Err("The key cannot start with a dash.");
        }
        steps.push(if key.is_empty() { unset_step("user.signingkey") } else { set_step("user.signingkey", &key) });
    }
    if let Some(enabled) = change.enabled {
        steps.push(match enabled {
            None => unset_step("commit.gpgsign"),
            Some(on) => set_step("commit.gpgsign", if on { "true" } else { "false" }),
        });
    }
    if steps.is_empty() {
        return Err("Nothing to change.");
    }
    Ok(steps)
}

pub fn set_signing(env: &Env, root: &Path, change: &SignChange) -> (Done, Option<Signing>) {
    let steps = match signing_steps(change) {
        Ok(s) => s,
        Err(problem) => return (refusal("invalid", problem, "Nothing was changed."), None),
    };
    for step in steps {
        let out = super::helper::git(env, root, &step.args);
        if !out.code.is_some_and(|c| step.ok_codes.contains(&c)) {
            return (failure(&out.said(), "git config failed"), None);
        }
    }
    (success("Saved the signing setting for this repo", ""), signing_info(env, root).ok())
}

pub fn enable_problem(info: &Signing, short_path: &dyn Fn(&str) -> String) -> Option<String> {
    if let Some(f) = &info.unknown_format {
        return Some(format!("gpg.format is \"{f}\", which git does not know. Pick SSH key or GPG key below first."));
    }
    if info.format == "ssh" && info.key.is_none() && info.key_command.is_none() {
        return Some("Pick a key first. Use one from SSH keys below, or type the path of your public key.".into());
    }
    if info.key_found == Some(false) {
        let key = short_path(info.key.as_deref().unwrap_or(""));
        return Some(format!("The key file {key} does not exist. Pick another key first."));
    }
    if info.program_found == Some(false) {
        return Some(format!(
            "{} is not installed, so signing would fail. Install it first, or sign with an SSH key.",
            info.program
        ));
    }
    None
}

pub fn signing_status(info: &Signing, short_path: &dyn Fn(&str) -> String) -> String {
    if !info.enabled {
        return "Your commits in this repo are not signed.".into();
    }
    let key = info.key.as_deref().filter(|_| !info.literal_key).map(short_path);
    match (info.format.as_str(), key) {
        ("ssh", _) if info.literal_key => "Your commits are signed with an SSH key that is written in the git config.".into(),
        ("ssh", Some(k)) => format!("Your commits are signed with the SSH key {k}."),
        ("ssh", None) if info.key_command.is_some() => "Your commits are signed with an SSH key that git asks for each time.".into(),
        ("ssh", None) => "Signing with an SSH key is on, but no key is set.".into(),
        ("x509", Some(k)) => format!("Your commits are signed with the X.509 certificate {k}."),
        ("x509", None) => "Your commits are signed with the X.509 certificate that matches your email.".into(),
        (_, Some(k)) => format!("Your commits are signed with the GPG key {k}."),
        (_, None) => "Your commits are signed with the GPG key that matches your email.".into(),
    }
}

pub fn signing_problems(info: &Signing, short_path: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(bad) = &info.bad_value {
        out.push(format!("commit.gpgsign is set to \"{bad}\", which git does not understand."));
    }
    if let Some(f) = &info.unknown_format {
        out.push(format!(
            "gpg.format is \"{f}\", which git does not know. Commits will fail until you pick SSH key or GPG key below."
        ));
    }
    if info.enabled && info.format == "ssh" && info.key.is_none() && info.key_command.is_none() {
        out.push("Commits will fail until you pick a key below.".into());
    }
    if info.enabled && info.key_found == Some(false) {
        let key = short_path(info.key.as_deref().unwrap_or(""));
        out.push(format!("The key file {key} does not exist, so commits will fail."));
    }
    if info.enabled && info.program_found == Some(false) {
        out.push(format!("{} is not installed, so commits will fail.", info.program));
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub path: String,
    pub name: String,
    pub url: Option<String>,
    pub sha: Option<String>,
    pub short: Option<String>,
    pub state: &'static str,
    pub describe: Option<String>,
    pub parent: Option<String>,
}

fn sub_state(mark: &str) -> &'static str {
    match mark {
        "+" => "modified",
        "-" => "uninitialized",
        "U" => "conflict",
        _ => "ok",
    }
}

pub fn sub_words(state: &str) -> (&'static str, &'static str) {
    match state {
        "modified" => ("On another commit", "warn"),
        "uninitialized" => ("Not downloaded", "warn"),
        "conflict" => ("Has a merge conflict", "bad"),
        _ => ("Up to date", "ok"),
    }
}

pub fn parse_submodules(text: &str, known: &[String]) -> Vec<Submodule> {
    let line_re = Regex::new(r"^([ +\-U])([0-9a-f]{40,64}) (.+)$").expect("submodule line");
    let desc_re = Regex::new(r"^(.+) \(([^()]+)\)$").expect("submodule describe");
    let zeros = Regex::new(r"^0+$").expect("zero sha");
    let mut out: Vec<Submodule> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        let Some(m) = line_re.captures(line) else {
            continue;
        };
        let mut path = m[3].to_string();
        let mut described = None;
        if !known.contains(&path)
            && let Some(d) = desc_re.captures(&path.clone())
        {
            path = d[1].to_string();
            described = Some(d[2].to_string());
        }
        let sha = (!zeros.is_match(&m[2])).then(|| m[2].to_string());
        out.push(Submodule {
            name: path.clone(),
            path,
            url: None,
            short: sha.as_ref().map(|s| s[..7].to_string()),
            sha,
            state: sub_state(&m[1]),
            describe: described,
            parent: None,
        });
    }
    let paths: Vec<String> = out.iter().map(|s| s.path.clone()).collect();
    for sub in out.iter_mut() {
        sub.parent = paths
            .iter()
            .filter(|p| **p != sub.path && sub.path.starts_with(&format!("{p}/")))
            .max_by_key(|p| p.len())
            .cloned();
    }
    out
}

pub fn parse_modules_file(text: &str) -> Vec<(String, String, Option<String>)> {
    let re = Regex::new(r"(?i)^submodule\.(.+)\.(path|url)$").expect("modules key");
    let mut by_name: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for entry in text.split('\0') {
        let Some(cut) = entry.find('\n') else {
            continue;
        };
        let Some(m) = re.captures(&entry[..cut]) else {
            continue;
        };
        let name = m[1].to_string();
        let value = entry[cut + 1..].to_string();
        let at = match by_name.iter().position(|(n, _, _)| *n == name) {
            Some(i) => i,
            None => {
                by_name.push((name, None, None));
                by_name.len() - 1
            }
        };
        if m[2].eq_ignore_ascii_case("path") {
            by_name[at].1 = Some(value);
        } else {
            by_name[at].2 = Some(value);
        }
    }
    by_name
        .into_iter()
        .filter_map(|(n, p, u)| p.map(|p| (n, p, u)))
        .collect()
}

pub fn list_submodules(env: &Env, root: &Path) -> Result<Vec<Submodule>, String> {
    let status = git_str(env, root, &["--no-optional-locks", "submodule", "status", "--recursive"]);
    if !status.ok() {
        let why = run::tail(&status.said(), 4);
        return Err(if why.is_empty() { "cannot read the submodules".into() } else { why });
    }
    let modules = git_str(
        env,
        root,
        &["config", "-z", "--file", ".gitmodules", "--get-regexp", r"^submodule\..*\.(path|url)$"],
    );
    let mods = if modules.ok() { parse_modules_file(&modules.stdout) } else { Vec::new() };
    let known: Vec<String> = mods.iter().map(|(_, p, _)| p.clone()).collect();
    Ok(parse_submodules(&status.stdout, &known)
        .into_iter()
        .map(|mut sub| {
            if sub.parent.is_none()
                && let Some((name, _, url)) = mods.iter().find(|(_, p, _)| *p == sub.path)
            {
                sub.name = name.clone();
                sub.url = url.clone();
            }
            sub
        })
        .collect())
}

pub fn update_submodules(env: &Env, root: &Path, cancel: Option<Arc<AtomicBool>>) -> Done {
    let mut all = env.git_env.clone();
    all.extend(run::GIT_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let out = run::run(
        &env.git,
        &run::strings(&["submodule", "update", "--init", "--recursive"]),
        &Opts {
            cwd: Some(root.to_path_buf()),
            env: all,
            timeout: Some(Duration::from_secs(1800)),
            cancel,
            ..Opts::default()
        },
    );
    if out.cancelled {
        return refusal("cancelled", "Stopped", "Some submodules may not be updated yet. Press Update all to finish.");
    }
    if out.timed_out {
        return failure("timed out waiting", "");
    }
    if !out.ok() {
        return failure(&out.said(), "git submodule update failed");
    }
    success("The submodules are up to date", "")
}

const AGE_LINE: &str = r#"m=$(stat -c %Y "$9" 2>/dev/null || stat -f %m "$9" 2>/dev/null); if [ -n "$m" ]; then printf 'lockAge=%s\n' "$(( $(date +%s) - m ))"; else echo lockAge=; fi"#;

fn stuck_script(git: &Path) -> String {
    let g = super::helper::quote(&git.display().to_string());
    [
        format!("x=$({g} rev-parse --git-path BISECT_LOG --git-path BISECT_START --git-path sequencer --git-path CHERRY_PICK_HEAD --git-path REVERT_HEAD --git-path rebase-merge --git-path rebase-apply --git-path MERGE_HEAD --git-path index.lock) || exit 3"),
        "here=$(pwd -P)".into(),
        "set -f".into(),
        "IFS='".into(),
        "'".into(),
        "set -- $x".into(),
        "unset IFS".into(),
        r#"if [ -e "$1" ] || [ -e "$2" ]; then echo bisect=1; printf 'bisectStart=%s\n' "$(head -n 1 "$2" 2>/dev/null)"; fi"#.into(),
        r#"if [ -d "$3" ]; then echo sequencer=1; if [ -e "$3/todo" ]; then printf 'seqOp=%s\n' "$(sed -n '/^[^#[:space:]]/{s/[[:space:]].*//;p;q;}' "$3/todo")"; printf 'seqTodo=%s\n' "$(grep -c '^[^#[:space:]]' "$3/todo")"; fi; fi"#.into(),
        r#"[ -e "$4" ] && echo pickHead=1"#.into(),
        r#"[ -e "$5" ] && echo revertHead=1"#.into(),
        r#"if [ -d "$6" ]; then echo rebase=merge; elif [ -d "$7" ]; then if [ -e "$7/applying" ]; then echo rebase=am; else echo rebase=apply; fi; fi"#.into(),
        r#"if [ -e "$8" ]; then printf 'mergeHead=%s\n' "$(head -n 1 "$8")"; fi"#.into(),
        format!(r#"if [ -e "$9" ]; then case $9 in /*) printf 'lockPath=%s\n' "$9";; *) printf 'lockPath=%s/%s\n' "$here" "$9";; esac; {AGE_LINE}; fi"#),
        "exit 0".into(),
    ]
    .join("\n")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stuck {
    pub bisect: Option<Option<String>>,
    pub sequencer: Option<(String, u32, bool)>,
    pub index_lock: Option<(String, Option<u64>)>,
    pub rebase_dir: Option<String>,
    pub merge_head: Option<Option<String>>,
}

impl Stuck {
    pub fn stuck(&self) -> bool {
        self.bisect.is_some() || self.sequencer.is_some() || self.index_lock.is_some()
    }
}

pub fn parse_stuck(text: &str) -> Stuck {
    let f = flags(text);
    let has = |k: &str| f.get(k).is_some_and(|v| v == "1");
    let running = has("pickHead") || has("revertHead");
    let op = match f.get("seqOp").map(String::as_str) {
        Some("pick" | "p") => Some("cherry-pick"),
        Some("revert") => Some("revert"),
        _ if has("revertHead") => Some("revert"),
        _ if has("pickHead") => Some("cherry-pick"),
        _ => None,
    };
    let age = f.get("lockAge").filter(|v| !v.is_empty()).and_then(|v| v.parse::<i64>().ok());
    Stuck {
        bisect: has("bisect").then(|| f.get("bisectStart").map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
        sequencer: has("sequencer").then(|| {
            let todo = f.get("seqTodo").and_then(|v| v.parse().ok()).unwrap_or(0);
            (op.unwrap_or("cherry-pick").to_string(), todo, running)
        }),
        index_lock: f
            .get("lockPath")
            .filter(|p| !p.is_empty())
            .map(|p| (p.clone(), age.map(|a| a.max(0) as u64))),
        rebase_dir: f.get("rebase").filter(|v| !v.is_empty()).cloned(),
        merge_head: f
            .get("mergeHead")
            .map(|v| Some(v.trim().to_string()).filter(|s| !s.is_empty())),
    }
}

pub fn stuck_state(env: &Env, root: &Path) -> Result<Stuck, String> {
    let out = sh(env, &stuck_script(&env.git), &[], Some(root));
    if !out.ok() {
        let why = run::tail(&out.stderr, 4);
        return Err(if why.is_empty() { "cannot read the repo state".into() } else { why });
    }
    Ok(parse_stuck(&out.stdout))
}

const SKIP_VALUE: &[&str] = &["-c", "-C", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--config-env"];

const GIT_VERBS: &[&str] = &[
    "fetch", "pull", "push", "status", "commit", "merge", "rebase", "checkout", "switch", "add", "gc", "maintenance",
    "clone", "submodule", "stash", "log", "diff", "reset", "cherry-pick", "revert", "am", "apply", "lfs", "blame",
    "show", "worktree", "tag", "branch", "remote", "ls-files", "update-index", "read-tree", "write-tree",
    "hash-object", "rm", "mv", "restore", "bisect", "grep", "init", "config", "notes", "fsck", "prune", "repack",
    "rev-parse", "rev-list", "cat-file", "for-each-ref", "ls-remote", "ls-tree", "diff-index", "diff-tree",
    "update-ref", "upload-pack", "receive-pack", "pack-objects", "index-pack", "sparse-checkout", "describe",
    "format-patch",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub command: String,
    pub label: String,
}

fn process_label(base: &str, words: &[&str], first: bool) -> String {
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if SKIP_VALUE.contains(&w) {
            i += 2;
            continue;
        }
        if base == "git" {
            if GIT_VERBS.contains(&w) {
                return format!("git {w}");
            }
        } else if !w.starts_with('-') {
            let plain = w.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
            return if first && plain { format!("{base} {w}") } else { base.to_string() };
        }
        i += 1;
    }
    base.to_string()
}

pub fn parse_processes(text: &str) -> Vec<Process> {
    let line_re = Regex::new(r"^\s*(\d+)\s+(.+?)\s*$").expect("ps line");
    let prog = Regex::new(r"(?:^|/)(git(?:-[\w-]+)?)(\s|$)").expect("git program");
    text.lines()
        .filter_map(|line| {
            let m = line_re.captures(line)?;
            let command = m[2].to_string();
            let found = prog.captures(&command)?;
            if command.contains("fsmonitor--daemon") {
                return None;
            }
            let name = found.get(1)?;
            let first = !command[..name.start()].contains(char::is_whitespace);
            let words: Vec<&str> = command[name.end()..].split_whitespace().collect();
            Some(Process {
                pid: m[1].parse().ok()?,
                label: process_label(name.as_str(), &words, first),
                command: command.clone(),
            })
        })
        .collect()
}

pub fn parse_cwds(text: &str) -> BTreeMap<u32, String> {
    text.lines()
        .filter_map(|l| {
            let (pid, cwd) = l.split_once('\t')?;
            Some((pid.parse().ok()?, cwd.trim().to_string()))
        })
        .collect()
}

fn trim_root(path: &str) -> String {
    let t = path.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.into() }
}

pub fn busy_processes(procs: &[Process], cwds: &BTreeMap<u32, String>, paths: &[String]) -> Vec<Process> {
    let mut roots: Vec<String> = Vec::new();
    for p in paths.iter().filter(|p| !p.is_empty()).map(|p| trim_root(p)) {
        if p != "/" && !roots.contains(&p) {
            roots.push(p);
        }
    }
    let inside = |dir: &str| roots.iter().any(|r| dir == r || dir.starts_with(&format!("{r}/")));
    procs
        .iter()
        .filter(|p| match cwds.get(&p.pid) {
            None => false,
            Some(cwd) if cwd.is_empty() => true,
            Some(cwd) => inside(&trim_root(cwd)) || roots.iter().any(|r| p.command.contains(r.as_str())),
        })
        .cloned()
        .collect()
}

const CWD_SCRIPT: &str = r#"for p in "$@"; do
  c=
  if [ -d "/proc/$p" ]; then c=$(readlink "/proc/$p/cwd" 2>/dev/null)
  elif [ -x /usr/sbin/lsof ]; then c=$(/usr/sbin/lsof -a -p "$p" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)
  fi
  if [ -z "$c" ]; then ps -p "$p" -o pid= >/dev/null 2>&1 || continue; fi
  printf '%s\t%s\n' "$p" "$c"
done
exit 0"#;

fn repo_paths(env: &Env, root: &Path) -> Vec<String> {
    let out = git_str(env, root, &["rev-parse", "--show-toplevel", "--git-common-dir"]);
    let got = if out.ok() { run::lines(&out.stdout) } else { Vec::new() };
    let top = got.first().cloned().unwrap_or_default();
    let common = got.get(1).cloned().unwrap_or_default();
    let common = if !common.is_empty() && !common.starts_with('/') && !top.is_empty() {
        format!("{top}/{common}")
    } else {
        common
    };
    [trim_root(&root.display().to_string()), top, common]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect()
}

pub fn running_git(env: &Env, root: &Path) -> Result<Vec<Process>, String> {
    let ps = run::run(
        Path::new("/bin/ps"),
        &run::strings(&["-Ao", "pid=,command="]),
        &Opts {
            timeout: Some(Duration::from_secs(10)),
            ..Opts::default()
        },
    );
    if !ps.ok() {
        let why = run::tail(&ps.stderr, 4);
        return Err(if why.is_empty() { "cannot list the running programs".into() } else { why });
    }
    let me = std::process::id();
    let procs: Vec<Process> = parse_processes(&ps.stdout).into_iter().filter(|p| p.pid != me).collect();
    if procs.is_empty() {
        return Ok(Vec::new());
    }
    let pids: Vec<String> = procs.iter().map(|p| p.pid.to_string()).collect();
    let pid_refs: Vec<&str> = pids.iter().map(String::as_str).collect();
    let cwd = sh(env, CWD_SCRIPT, &pid_refs, None);
    Ok(busy_processes(&procs, &parse_cwds(&cwd.stdout), &repo_paths(env, root)))
}

const UNLOCK_SCRIPT: &str = r#"f=$1
min=$2
case $f in */index.lock) ;; *) exit 5;; esac
[ -e "$f" ] || exit 0
m=$(stat -c %Y "$f" 2>/dev/null || stat -f %m "$f" 2>/dev/null)
[ -n "$m" ] || exit 6
a=$(( $(date +%s) - m ))
[ "$a" -ge "$min" ] || exit 4
rm -f -- "$f""#;

pub fn age_words(sec: u64) -> String {
    let plural = |n: u64, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    if sec < 60 {
        return plural(sec, "second");
    }
    let min = sec / 60;
    if min < 60 {
        return plural(min, "minute");
    }
    let hours = min / 60;
    if hours < 48 {
        return plural(hours, "hour");
    }
    format!("{} days", hours / 24)
}

fn other_running(state: &Stuck) -> Option<&'static str> {
    if let Some((op, _, true)) = &state.sequencer {
        return Some(if op == "revert" {
            "A revert stopped on a conflict"
        } else {
            "A cherry-pick stopped on a conflict"
        });
    }
    if let Some(kind) = &state.rebase_dir {
        return Some(if kind == "am" { "A git am is in progress" } else { "A rebase is in progress" });
    }
    if state.merge_head.is_some() {
        return Some("A merge is in progress");
    }
    None
}

pub fn clear_stuck(env: &Env, root: &Path, what: &str) -> Done {
    if !["bisect", "sequencer", "indexLock"].contains(&what) {
        return refusal("invalid", "Nothing to fix there", "Pick bisect, sequencer or indexLock.");
    }
    let state = match stuck_state(env, root) {
        Ok(s) => s,
        Err(e) => return failure(&e, "cannot read the repo state"),
    };
    match what {
        "indexLock" => match &state.index_lock {
            None => return success("The lock file is already gone", ""),
            Some((_, age)) if age.is_none_or(|a| a < LOCK_MIN_AGE) => {
                let hint = format!(
                    "The lock file is only {} old. Wait for the command to end. It was not removed.",
                    age_words(age.unwrap_or(0))
                );
                return refusal("fresh", "A git command is working now", &hint);
            }
            _ => {}
        },
        "bisect" if state.bisect.is_none() => return success("No bisect is going on", ""),
        "sequencer" if state.sequencer.is_none() => return success("Nothing is left over", ""),
        _ => {}
    }
    if what != "indexLock"
        && let Some(other) = other_running(&state)
    {
        return refusal("running", other, "Finish or stop it in the Changes panel first. Nothing was changed.");
    }
    match running_git(env, root) {
        Ok(busy) if !busy.is_empty() => {
            let mut done = refusal("busy", "A git command is running in this repo", "Wait for it to end, then try again.");
            done.detail = busy
                .iter()
                .take(3)
                .map(|p| format!("{} (process {})", p.label, p.pid))
                .collect::<Vec<_>>()
                .join("\n");
            return done;
        }
        Err(e) => return failure(&e, "cannot list the running programs"),
        _ => {}
    }
    if what == "indexLock" {
        let path = state.index_lock.map(|(p, _)| p).unwrap_or_default();
        let out = sh(env, UNLOCK_SCRIPT, &[&path, &LOCK_MIN_AGE.to_string()], Some(root));
        if out.code == Some(4) {
            return refusal(
                "fresh",
                "A git command is working now",
                "The lock file was made again just now. Wait for the command to end.",
            );
        }
        if !out.ok() {
            return failure(&out.said(), "could not remove index.lock");
        }
        return success("Removed the lock file", "Git can change this repo again.");
    }
    if what == "bisect" {
        let out = git_str(env, root, &["bisect", "reset"]);
        if !out.ok() {
            return failure(&out.said(), "git bisect reset failed");
        }
        let start = state.bisect.flatten().unwrap_or_default();
        let start = start.trim_start_matches("refs/heads/");
        let sha = Regex::new(r"^[0-9a-f]{40,64}$").expect("sha");
        let place = if sha.is_match(start) { format!("commit {}", &start[..7]) } else { start.to_string() };
        let hint = if place.is_empty() { String::new() } else { format!("You are back on {place}.") };
        return success("Stopped the bisect", &hint);
    }
    let op = match &state.sequencer {
        Some((op, _, _)) if op == "revert" => "revert",
        _ => "cherry-pick",
    };
    let out = git_str(env, root, &[op, "--quit"]);
    if !out.ok() {
        return failure(&out.said(), &format!("git {op} --quit failed"));
    }
    let title = if op == "revert" { "Cleared the left-over revert" } else { "Cleared the left-over cherry-pick" };
    success(title, "Your files and commits stay as they are.")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKey {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub bits: u32,
    pub fingerprint: String,
    pub comment: String,
    pub algo: String,
    pub public_key: String,
    pub in_agent: Option<bool>,
}

pub fn parse_fingerprint(line: &str) -> Option<(u32, String, String, String)> {
    let re = Regex::new(r"^(\d+)\s+(\S+)\s+(.*?)\s*\(([^()]+)\)\s*$").expect("fingerprint");
    let m = re.captures(line.trim())?;
    let comment = m[3].trim();
    Some((
        m[1].parse().ok()?,
        m[2].to_string(),
        if comment == "no comment" { String::new() } else { comment.to_string() },
        m[4].trim().to_string(),
    ))
}

const SSH_SCRIPT: &str = r#"h=${1:-$HOME}
d="$h/.ssh"
[ -d "$d" ] || exit 0
for f in "$d"/*.pub; do
  [ -f "$f" ] || continue
  s=$(wc -c < "$f" | tr -d " ")
  [ "$s" -gt 0 ] && [ "$s" -le 16384 ] || continue
  grep -q "PRIVATE KEY" "$f" && continue
  k=$(sed -n '/^[^#[:space:]]/{p;q;}' "$f")
  [ -n "$k" ] || continue
  l=$(ssh-keygen -l -f "$f" 2>/dev/null | head -n 1)
  printf '%s\t%s\t%s\n' "$f" "$l" "$k"
done
exit 0"#;

pub fn parse_ssh_keys(text: &str) -> Vec<SshKey> {
    let mut out: Vec<SshKey> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let path = parts.next().unwrap_or("");
            let finger = parts.next().unwrap_or("");
            let key = parts.next().unwrap_or("").trim();
            let (bits, fingerprint, comment, kind) = parse_fingerprint(finger)?;
            if path.is_empty() || key.is_empty() {
                return None;
            }
            let algo = key
                .split_whitespace()
                .find(|w| literal_ssh_key(w) && !w.starts_with("key::"))
                .or(key.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            Some(SshKey {
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                path: path.to_string(),
                kind,
                bits,
                fingerprint,
                comment,
                algo,
                public_key: key.to_string(),
                in_agent: None,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn agent_fingerprints(env: &Env) -> Option<Vec<String>> {
    let out = sh(env, "ssh-add -l", &[], None);
    match out.code {
        Some(1) => Some(Vec::new()),
        Some(0) => Some(
            run::lines(&out.stdout)
                .iter()
                .filter_map(|l| parse_fingerprint(l))
                .map(|f| f.1)
                .collect(),
        ),
        _ => None,
    }
}

pub fn ssh_keys(env: &Env) -> Result<Vec<SshKey>, String> {
    let home = env.home.display().to_string();
    let out = sh(env, SSH_SCRIPT, &[&home], None);
    if !out.ok() {
        let why = run::tail(&out.stderr, 4);
        return Err(if why.is_empty() { "cannot read the SSH keys".into() } else { why });
    }
    let mut keys = parse_ssh_keys(&out.stdout);
    if !keys.is_empty() {
        let agent = agent_fingerprints(env);
        for k in keys.iter_mut() {
            k.in_agent = agent.as_ref().map(|list| list.contains(&k.fingerprint));
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testkit;

    #[test]
    fn signing_is_read_and_set() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("app");
        let info = signing_info(env, &repo).unwrap();
        assert!(!info.enabled);
        assert_eq!(info.format, "openpgp");
        assert_eq!(signing_status(&info, &|p| p.to_string()), "Your commits in this repo are not signed.");
        let change = SignChange {
            format: Some(Some("ssh".into())),
            key: Some(Some("~/.ssh/none.pub".into())),
            enabled: Some(Some(true)),
        };
        let (done, info) = set_signing(env, &repo, &change);
        assert!(done.ok, "{done:?}");
        let info = info.unwrap();
        assert!(info.enabled);
        assert_eq!(info.format, "ssh");
        assert_eq!(info.key_scope, "local");
        assert_eq!(info.key_found, Some(false));
        let problems = signing_problems(&info, &|p| p.to_string());
        assert!(problems.iter().any(|p| p.contains("does not exist")));
        let bad = SignChange {
            key: Some(Some("-x".into())),
            ..SignChange::default()
        };
        assert_eq!(signing_steps(&bad), Err("The key cannot start with a dash."));
        assert_eq!(signing_steps(&SignChange::default()), Err("Nothing to change."));
    }

    #[test]
    fn config_and_bool_parsing() {
        assert_eq!(git_bool(None), Some(true));
        assert_eq!(git_bool(Some("Yes")), Some(true));
        assert_eq!(git_bool(Some("0")), Some(false));
        assert_eq!(git_bool(Some("maybe")), None);
        let e = parse_scoped_config("global\0commit.gpgsign\ntrue\0local\0gpg.format\nssh\0", false);
        assert_eq!(e.len(), 2);
        let s = build_signing(&e);
        assert!(s.enabled);
        assert_eq!(s.scope, "global");
        assert_eq!(s.program, "ssh-keygen");
        let odd = build_signing(&parse_scoped_config("local\0commit.gpgsign\nmaybe\0local\0gpg.format\nfoo\0", false));
        assert_eq!(odd.bad_value.as_deref(), Some("maybe"));
        assert_eq!(odd.unknown_format.as_deref(), Some("foo"));
    }

    #[test]
    fn submodules_and_stuck_and_keys_parse() {
        let sha = "a".repeat(40);
        let text = format!("+{sha} lib (v1)\n-{} lib/inner\n", "0".repeat(40));
        let subs = parse_submodules(&text, &[]);
        assert_eq!(subs[0].path, "lib");
        assert_eq!(subs[0].describe.as_deref(), Some("v1"));
        assert_eq!(subs[0].state, "modified");
        assert_eq!(subs[1].sha, None);
        assert_eq!(subs[1].parent.as_deref(), Some("lib"));
        let mods = parse_modules_file("submodule.lib.path\nlib\0submodule.lib.url\nhttps://x/y\0");
        assert_eq!(mods, [("lib".to_string(), "lib".to_string(), Some("https://x/y".to_string()))]);
        let s = parse_stuck("bisect=1\nbisectStart=main\nlockPath=/r/.git/index.lock\nlockAge=5\n");
        assert!(s.stuck());
        assert_eq!(s.bisect, Some(Some("main".into())));
        assert_eq!(s.index_lock, Some(("/r/.git/index.lock".into(), Some(5))));
        assert_eq!(age_words(1), "1 second");
        assert_eq!(age_words(3600 * 50), "2 days");
        let procs = parse_processes(" 12 /usr/bin/git -C /r fetch origin\n 13 /bin/zsh\n 14 git-remote-https origin x\n");
        assert_eq!(procs[0].label, "git fetch");
        assert_eq!(procs[1].label, "git-remote-https origin");
        let cwds = parse_cwds("12\t/r/sub\n14\t/else\n");
        let busy = busy_processes(&procs, &cwds, &["/r".into()]);
        assert_eq!(busy.len(), 1);
        let keys = parse_ssh_keys("/h/.ssh/id.pub\t256 SHA256:abc me@box (ED25519)\tssh-ed25519 AAAA me@box\n");
        assert_eq!(keys[0].name, "id.pub");
        assert_eq!(keys[0].algo, "ssh-ed25519");
        assert_eq!(keys[0].kind, "ED25519");
    }

    #[test]
    fn stuck_state_and_clear_work_on_a_real_repo() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("app");
        let state = stuck_state(env, &repo).unwrap();
        assert!(!state.stuck());
        std::fs::write(repo.join(".git/index.lock"), "").unwrap();
        let state = stuck_state(env, &repo).unwrap();
        assert!(state.index_lock.is_some());
        let done = clear_stuck(env, &repo, "indexLock");
        assert_eq!(done.kind, "fresh");
        assert!(repo.join(".git/index.lock").exists());
        assert!(clear_stuck(env, &repo, "bisect").ok);
        assert_eq!(clear_stuck(env, &repo, "odd").kind, "invalid");
    }
}
