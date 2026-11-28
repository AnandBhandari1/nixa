mod json;
mod merge;
mod omp;
mod place;
mod session;

use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::help;
use anyhow::{Context, Result, anyhow, bail};
use json::Json;
use serde_json::Value;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Agent {
    Claude,
    Codex,
    Omp,
    Cursor,
    Agy,
}

const ALL: [Agent; 5] = [Agent::Claude, Agent::Codex, Agent::Omp, Agent::Cursor, Agent::Agy];
const AGY_GROUP: &str = "nixa-status";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Install,
    Remove,
    Status,
}

struct Setup {
    home: Option<PathBuf>,
    hooks: PathBuf,
    script: String,
    apply: bool,
    json: bool,
}

fn name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Omp => "omp",
        Agent::Cursor => "cursor",
        Agent::Agy => "agy",
    }
}

fn label(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude Code",
        Agent::Codex => "Codex",
        Agent::Omp => "omp",
        Agent::Cursor => "Cursor",
        Agent::Agy => "Antigravity",
    }
}

fn parse_agent(word: &str) -> Result<Vec<Agent>> {
    Ok(match word {
        "all" => ALL.to_vec(),
        "claude" | "claude-code" => vec![Agent::Claude],
        "codex" => vec![Agent::Codex],
        "omp" => vec![Agent::Omp],
        "cursor" | "cursor-agent" => vec![Agent::Cursor],
        "agy" | "antigravity" => vec![Agent::Agy],
        other => return Err(usage(format!("unknown agent {other:?}, use claude, codex, omp, cursor, agy or all"))),
    })
}

fn pick_agents(words: &[String]) -> Result<Vec<Agent>> {
    if words.is_empty() {
        return Ok(ALL.to_vec());
    }
    let mut picked = Vec::new();
    for word in words {
        for agent in parse_agent(word)? {
            if !picked.contains(&agent) {
                picked.push(agent);
            }
        }
    }
    Ok(picked)
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::HOOKS.as_bytes())?;
        return Ok(2);
    };
    let action = match sub.as_str() {
        "install" | "add" => Action::Install,
        "remove" | "rm" | "uninstall" => Action::Remove,
        "status" | "ls" => Action::Status,
        "session" => return session::run(ctx, rest, out),
        "help" | "-h" | "--help" => {
            out.write_all(help::HOOKS.as_bytes())?;
            return Ok(0);
        }
        other => return Err(usage(format!("unknown command nixa hooks {other}"))),
    };
    let (mut p, ctx) = ctx::command(ctx, rest, &["--home", "--hooks"], false)?;
    let apply = p.flag(&["--apply"]);
    if apply && action == Action::Status {
        return Err(usage("nixa hooks status takes no --apply"));
    }
    let home = p.value(&["--home"]).map(PathBuf::from);
    let hooks_dir = match p.value(&["--hooks"]) {
        Some(dir) => PathBuf::from(dir),
        None => default_hooks()?,
    };
    p.finish(usize::MAX, false)?;
    let agents = pick_agents(&p.pos)?;
    let hooks = fs::canonicalize(&hooks_dir).with_context(|| format!("cannot find the hooks folder {}", hooks_dir.display()))?;
    let script = hooks.join(merge::SCRIPT);
    if !script.is_file() {
        bail!("{} is missing", script.display());
    }
    let setup = Setup { home, script: script.display().to_string(), hooks, apply, json: ctx.json };
    let mut code = 0;
    let mut report = Vec::new();
    for agent in agents {
        match one(&setup, agent, action, out) {
            Ok(v) => report.push(v),
            Err(e) => {
                code = 1;
                if setup.json {
                    report.push(serde_json::json!({"agent": name(agent), "error": format!("{e:#}")}));
                } else {
                    writeln!(out, "{}: not changed", label(agent))?;
                    eprintln!("nixa: {}: {e:#}", label(agent));
                }
            }
        }
    }
    if setup.json {
        writeln!(out, "{}", Value::Array(report))?;
    } else if action != Action::Status && !apply {
        writeln!(out, "Dry run: nothing was written. Add --apply to write it. Each changed file is backed up first.")?;
    }
    Ok(code)
}

fn hooks_near(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .take(5)
        .flat_map(|dir| [dir.join("Resources").join("hooks"), dir.join("hooks")])
        .find(|dir| dir.join(merge::SCRIPT).is_file())
}

fn default_hooks() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot find the nixa command path")?;
    hooks_near(&place::real_path(&exe))
        .ok_or_else(|| anyhow!("cannot find the hooks folder next to {}, give --hooks DIR", exe.display()))
}

fn agent_dir(setup: &Setup, agent: Agent) -> Result<PathBuf> {
    let (env, rel) = match agent {
        Agent::Claude => (Some("CLAUDE_CONFIG_DIR"), ".claude"),
        Agent::Codex => (Some("CODEX_HOME"), ".codex"),
        Agent::Omp => (None, ".omp/agent"),
        Agent::Cursor => (None, ".cursor"),
        Agent::Agy => (None, ".gemini/config"),
    };
    if let Some(home) = &setup.home {
        return Ok(home.join(rel));
    }
    if let Some(dir) = env.and_then(std::env::var_os).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("HOME is not set, give --home DIR"))?;
    Ok(PathBuf::from(home).join(rel))
}

fn one(setup: &Setup, agent: Agent, action: Action, out: &mut dyn Write) -> Result<Value> {
    let dir = agent_dir(setup, agent)?;
    if !dir.is_dir() {
        if !setup.json {
            writeln!(out, "{}: not found ({} does not exist), skipped", label(agent), dir.display())?;
        }
        return Ok(serde_json::json!({"agent": name(agent), "found": false, "dir": dir}));
    }
    match agent {
        Agent::Omp => omp::run(setup, &dir, action, out),
        Agent::Claude | Agent::Codex | Agent::Cursor | Agent::Agy => config(setup, agent, &dir, action, out),
    }
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn template(setup: &Setup, agent: Agent) -> Result<Json> {
    let rel = match agent {
        Agent::Claude => "claude/settings-hooks.json",
        Agent::Cursor => "cursor/hooks.json",
        Agent::Agy => "agy/hooks.json",
        _ => "codex/hooks.json",
    };
    let path = setup.hooks.join(rel);
    let text = fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut t = json::parse(&text).with_context(|| format!("{} is not valid JSON", path.display()))?;
    merge::rewrite(&mut t, &setup.script).with_context(|| format!("bad hook template {}", path.display()))?;
    Ok(t)
}

fn config(setup: &Setup, agent: Agent, dir: &Path, action: Action, out: &mut dyn Write) -> Result<Value> {
    let file = dir.join(if agent == Agent::Claude { "settings.json" } else { "hooks.json" });
    let template = template(setup, agent)?;
    let old_text = read_optional(&file)?;
    let whole = match old_text.as_deref() {
        Some(t) if !t.trim().is_empty() => {
            json::parse(t).with_context(|| format!("{} is not valid JSON", file.display()))?
        }
        _ => Json::Obj(Vec::new()),
    };
    let old = if agent == Agent::Agy { agy_view(&whole)? } else { whole.clone() };
    let want = merge::ours(&template, &setup.script);
    let have = merge::ours(&old, &setup.script);
    let others = merge::others(&old, &setup.script);
    let say = !setup.json;
    if say {
        writeln!(out, "{}: {}", label(agent), file.display())?;
    }
    if action == Action::Status {
        let missing = merge::minus(&want, &have);
        let extra = merge::minus(&have, &want);
        let state = if have.is_empty() {
            "not installed"
        } else if missing.is_empty() && extra.is_empty() {
            "installed"
        } else {
            "out of date"
        };
        if say {
            writeln!(out, "  {state}, {} of {} Nixa hooks", want.len() - missing.len(), want.len())?;
            if state == "out of date" {
                writeln!(out, "  run nixa hooks install {} to update them", name(agent))?;
            }
            for other in &others {
                writeln!(out, "  other nixa-status.sh hook (another folder): {other}")?;
            }
        }
        return Ok(serde_json::json!({
            "agent": name(agent),
            "found": true,
            "file": file,
            "state": state,
            "have": want.len() - missing.len(),
            "want": want.len(),
            "others": others,
        }));
    }
    let mut new = old.clone();
    merge::remove(&mut new, &setup.script);
    if action == Action::Install {
        if agent == Agent::Cursor && matches!(new, Json::Obj(_)) {
            json::entry(&mut new, "version", Json::Num("1".to_string()));
        }
        merge::add(&mut new, &template).with_context(|| format!("cannot merge into {}", file.display()))?;
    }
    let after = merge::ours(&new, &setup.script);
    let added = merge::minus(&after, &have);
    let removed = merge::minus(&have, &after);
    let changed = have != after;
    let new = if agent == Agent::Agy { agy_back(whole.clone(), new) } else { new };
    let new_text = json::pretty(&new);
    let reformat = old_text.as_deref().is_some_and(|t| !t.trim().is_empty() && json::pretty(&whole) != t);
    if say {
        if !changed {
            writeln!(out, "  no change")?;
        }
        for line in &removed {
            writeln!(out, "  - {line}")?;
        }
        for line in &added {
            writeln!(out, "  + {line}")?;
        }
        if changed && added.is_empty() && removed.is_empty() {
            writeln!(out, "  ~ the Nixa hooks move to the end of their lists")?;
        }
        if changed && reformat {
            writeln!(out, "  note: the file is written back as JSON with 2 space indent; other keys keep their order and values")?;
        }
    }
    let written = changed && setup.apply;
    let mut backup = None;
    if written {
        backup = place::write(&file, &new_text)?;
        if say {
            if let Some(b) = &backup {
                writeln!(out, "  backup: {}", b.display())?;
            }
            writeln!(out, "  written")?;
        }
    }
    if say && changed && agent == Agent::Codex && action == Action::Install {
        writeln!(out, "  then open Codex, run /hooks and trust the new hooks")?;
    }
    if say && changed && agent == Agent::Agy && action == Action::Install {
        writeln!(out, "  then start agy again so it reads the new hooks")?;
    }
    if say && changed && agent == Agent::Cursor && action == Action::Install {
        writeln!(out, "  then restart Cursor and cursor-agent so they read the new hooks")?;
    }
    Ok(serde_json::json!({
        "agent": name(agent),
        "found": true,
        "file": file,
        "changed": changed,
        "written": written,
        "added": added,
        "removed": removed,
        "backup": backup,
    }))
}

fn agy_view(whole: &Json) -> Result<Json> {
    if !matches!(whole, Json::Obj(_)) {
        bail!("the file is not a JSON object");
    }
    Ok(match json::get(whole, AGY_GROUP) {
        Some(group @ Json::Obj(_)) => Json::Obj(vec![("hooks".to_string(), group.clone())]),
        Some(_) => bail!("\"{AGY_GROUP}\" in the file is not an object"),
        None => Json::Obj(Vec::new()),
    })
}

fn agy_back(mut whole: Json, view: Json) -> Json {
    match json::get(&view, "hooks") {
        Some(group) => {
            let group = group.clone();
            if let Some(slot) = json::entry(&mut whole, AGY_GROUP, Json::Obj(Vec::new())) {
                *slot = group;
            }
        }
        None => json::drop_key(&mut whole, AGY_GROUP),
    }
    whole
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_hooks_live_in_their_own_group_and_other_groups_stay() {
        let whole = json::parse(r#"{"speak":{"Stop":[{"type":"command","command":"say hi"}]}}"#).unwrap();
        let view = agy_view(&whole).unwrap();
        assert!(json::get(&view, "hooks").is_none());
        let mut new = view.clone();
        let template = json::parse(r#"{"hooks":{"Stop":[{"type":"command","command":"'/x/nixa-status.sh' done"}]}}"#).unwrap();
        merge::add(&mut new, &template).unwrap();
        let back = agy_back(whole.clone(), new.clone());
        assert_eq!(json::compact(&back), r#"{"speak":{"Stop":[{"type":"command","command":"say hi"}]},"nixa-status":{"Stop":[{"type":"command","command":"'/x/nixa-status.sh' done"}]}}"#);
        let mut gone = agy_view(&back).unwrap();
        merge::remove(&mut gone, "/x/nixa-status.sh");
        assert_eq!(json::compact(&agy_back(back, gone)), json::compact(&whole));
        assert!(agy_view(&json::parse(r#"{"nixa-status":[]}"#).unwrap()).is_err());
    }

    #[test]
    fn hooks_are_found_next_to_the_command() {
        let root = std::env::temp_dir().join(format!("nixa-hooks-near-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repo_hooks = root.join("repo/hooks");
        let bundle_hooks = root.join("Nixa.app/Contents/Resources/hooks");
        for dir in [&repo_hooks, &bundle_hooks] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join(merge::SCRIPT), "").unwrap();
        }
        assert_eq!(hooks_near(&root.join("repo/target/gpui/debug/nixa")), Some(repo_hooks));
        assert_eq!(hooks_near(&root.join("Nixa.app/Contents/MacOS/nixa")), Some(bundle_hooks));
        assert_eq!(hooks_near(&root.join("other/nixa")), None);
        fs::remove_dir_all(&root).ok();
    }
}
