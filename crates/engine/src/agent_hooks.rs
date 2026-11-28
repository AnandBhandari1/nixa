use std::path::{Path, PathBuf};

const SCRIPT: &str = "nixa-status.sh";
const TEMPLATE_SCRIPT: &str = "./hooks/nixa-status.sh";
const OMP_EXTENSION: &str = "nixa-status.ts";
const AGY_DIR: &str = "agy-hooks";

pub fn extra_args(cmd: &[String]) -> Vec<String> {
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| hooks_dir(&exe)) else {
        return Vec::new();
    };
    let mut args = claude_args(cmd, &dir);
    args.extend(codex_args(cmd, &dir));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    args.extend(omp_args(cmd, &dir, home.as_deref()));
    args.extend(agy_args(cmd, &dir, &nixa_protocol::paths::data_dir(), home.as_deref()));
    args
}

pub fn omp_args(cmd: &[String], hooks: &Path, home: Option<&Path>) -> Vec<String> {
    let Some((prog, rest)) = cmd.split_first() else {
        return Vec::new();
    };
    if Path::new(prog).file_name().is_none_or(|n| n != "omp") {
        return Vec::new();
    }
    let ext = hooks.join("omp").join(OMP_EXTENSION);
    let sub = rest.first().is_some_and(|a| !a.starts_with('-') && !a.contains(char::is_whitespace));
    let linked = home.is_some_and(|h| h.join(".omp/agent/extensions").join(OMP_EXTENSION).exists());
    let given = rest.iter().any(|a| {
        a == "--" || a.ends_with(OMP_EXTENSION) || a == "--no-tools" || a.starts_with("--trusted-extension")
    });
    if sub || linked || given || !ext.is_file() {
        return Vec::new();
    }
    vec!["-e".into(), ext.display().to_string()]
}

pub fn agy_args(cmd: &[String], hooks: &Path, data: &Path, home: Option<&Path>) -> Vec<String> {
    let Some((prog, rest)) = cmd.split_first() else {
        return Vec::new();
    };
    if Path::new(prog).file_name().is_none_or(|n| n != "agy") {
        return Vec::new();
    }
    let sub = rest.first().is_some_and(|a| !a.starts_with('-'));
    let installed = home.and_then(|h| std::fs::read_to_string(h.join(".gemini/config/hooks.json")).ok()).is_some_and(|t| t.contains(SCRIPT));
    if sub || installed {
        return Vec::new();
    }
    let Some(hooks_json) = filled_template(&hooks.join("agy").join("hooks.json"), &hooks.join(SCRIPT)) else {
        return Vec::new();
    };
    let dir = std::path::absolute(data.join(AGY_DIR)).unwrap_or_else(|_| data.join(AGY_DIR));
    let file = dir.join(".agents").join("hooks.json");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(hooks_json.as_str()) {
        let part = dir.join(".agents").join(format!("hooks.json.{}", std::process::id()));
        let saved = std::fs::create_dir_all(dir.join(".agents"))
            .and_then(|_| std::fs::write(&part, &hooks_json))
            .and_then(|_| std::fs::rename(&part, &file));
        if let Err(error) = saved {
            eprintln!("nixa-engine: cannot write the agy status hooks: {error}");
            return Vec::new();
        }
    }
    vec!["--add-dir".into(), dir.display().to_string()]
}

fn filled_template(template: &Path, script: &Path) -> Option<String> {
    let template = std::fs::read_to_string(template).ok()?;
    let script = script.display().to_string();
    let quoted = format!("'{}'", script.replace('\'', "'\\''"));
    let quoted_json = serde_json::to_string(&quoted).ok()?;
    let filled = template.replace(TEMPLATE_SCRIPT, &quoted_json[1..quoted_json.len() - 1]);
    serde_json::from_str::<serde_json::Value>(&filled).ok()?;
    Some(filled)
}

pub fn codex_args(cmd: &[String], hooks: &Path) -> Vec<String> {
    let is_codex = cmd.first().is_some_and(|p| Path::new(p).file_name().is_some_and(|n| n == "codex"));
    if !is_codex || cmd.iter().any(|a| a == "--" || a.contains("notify=")) || !hooks.join(SCRIPT).is_file() {
        return Vec::new();
    }
    let notify = serde_json::json!(["/bin/sh", "-c", "exec \"$0\" done </dev/null", hooks.join(SCRIPT).display().to_string()]);
    vec!["-c".into(), format!("notify={notify}")]
}

pub fn hooks_dir(exe: &Path) -> Option<PathBuf> {
    let exe = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    exe.ancestors()
        .skip(1)
        .take(5)
        .flat_map(|dir| [dir.join("Resources").join("hooks"), dir.join("hooks")])
        .find(|dir| dir.join(SCRIPT).is_file())
}

pub fn claude_args(cmd: &[String], hooks: &Path) -> Vec<String> {
    let is_claude = cmd.first().is_some_and(|p| Path::new(p).file_name().is_some_and(|n| n == "claude"));
    if !is_claude || cmd.iter().any(|a| a == "--settings" || a.starts_with("--settings=")) {
        return Vec::new();
    }
    let Some(settings) = filled_template(&hooks.join("claude").join("settings-hooks.json"), &hooks.join(SCRIPT)) else {
        return Vec::new();
    };
    vec!["--settings".into(), settings]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hooks(dir: &Path) -> PathBuf {
        let hooks = dir.join("my hooks");
        std::fs::create_dir_all(hooks.join("claude")).unwrap();
        std::fs::write(hooks.join(SCRIPT), "").unwrap();
        std::fs::write(hooks.join("claude/settings-hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./hooks/nixa-status.sh done"}]}]}}"#).unwrap();
        hooks
    }

    #[test]
    fn claude_gets_status_hooks_with_the_full_script_path() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks(dir.path());
        let args = claude_args(&["/usr/local/bin/claude".into(), "--x".into()], &hooks);
        assert_eq!(args[0], "--settings");
        let v: serde_json::Value = serde_json::from_str(&args[1]).unwrap();
        let command = v["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(command, format!("'{}' done", hooks.join(SCRIPT).display()));
    }

    #[test]
    fn codex_gets_a_done_notify_with_the_full_script_path() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks(dir.path());
        let args = codex_args(&["/opt/bin/codex".into(), "resume".into()], &hooks);
        assert_eq!(args[0], "-c");
        let value: serde_json::Value = serde_json::from_str(args[1].strip_prefix("notify=").unwrap()).unwrap();
        assert_eq!(value[0], "/bin/sh");
        assert_eq!(value[3].as_str().unwrap(), hooks.join(SCRIPT).display().to_string());
        assert!(value[2].as_str().unwrap().contains("done"));
        assert!(codex_args(&["codex".into(), "-c".into(), "notify=[]".into()], &hooks).is_empty());
        assert!(codex_args(&["claude".into()], &hooks).is_empty());
    }

    #[test]
    fn omp_gets_the_status_extension_unless_it_is_linked_or_a_subcommand() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks(dir.path());
        std::fs::create_dir_all(hooks.join("omp")).unwrap();
        std::fs::write(hooks.join("omp").join(OMP_EXTENSION), "").unwrap();
        let home = dir.path().join("home");
        let ext = hooks.join("omp").join(OMP_EXTENSION).display().to_string();
        assert_eq!(omp_args(&["/opt/bin/omp".into()], &hooks, Some(&home)), ["-e".to_string(), ext.clone()]);
        assert_eq!(omp_args(&["omp".into(), "--model".into(), "opus".into()], &hooks, None), ["-e".to_string(), ext.clone()]);
        assert_eq!(omp_args(&["omp".into(), "fix the bug".into()], &hooks, None), ["-e".to_string(), ext]);
        assert!(omp_args(&["omp".into(), "config".into()], &hooks, None).is_empty());
        assert!(omp_args(&["omp".into(), "--".into(), "x".into()], &hooks, None).is_empty());
        assert!(omp_args(&["omp".into(), "--no-tools".into()], &hooks, None).is_empty());
        assert!(omp_args(&["claude".into()], &hooks, None).is_empty());
        std::fs::create_dir_all(home.join(".omp/agent/extensions")).unwrap();
        std::fs::write(home.join(".omp/agent/extensions").join(OMP_EXTENSION), "").unwrap();
        assert!(omp_args(&["omp".into()], &hooks, Some(&home)).is_empty());
    }

    #[test]
    fn agy_gets_a_hooks_folder_with_the_full_script_path_unless_installed() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks(dir.path());
        std::fs::create_dir_all(hooks.join("agy")).unwrap();
        std::fs::write(hooks.join("agy/hooks.json"), r#"{"hooks":{"Stop":[{"type":"command","command":"./hooks/nixa-status.sh done"}]}}"#).unwrap();
        let data = dir.path().join("data");
        let home = dir.path().join("home");
        let args = agy_args(&["/bin/agy".into(), "--model".into(), "x".into()], &hooks, &data, Some(&home));
        assert_eq!(args, ["--add-dir".to_string(), data.join(AGY_DIR).display().to_string()]);
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data.join(AGY_DIR).join(".agents/hooks.json")).unwrap()).unwrap();
        assert_eq!(written["hooks"]["Stop"][0]["command"].as_str().unwrap(), format!("'{}' done", hooks.join(SCRIPT).display()));
        assert!(agy_args(&["agy".into(), "mcp".into()], &hooks, &data, None).is_empty());
        assert!(agy_args(&["claude".into()], &hooks, &data, None).is_empty());
        std::fs::create_dir_all(home.join(".gemini/config")).unwrap();
        std::fs::write(home.join(".gemini/config/hooks.json"), format!("{{\"nixa-status\":\"{SCRIPT}\"}}")).unwrap();
        assert!(agy_args(&["agy".into()], &hooks, &data, Some(&home)).is_empty());
    }

    #[test]
    fn other_programs_and_own_settings_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks(dir.path());
        assert!(claude_args(&["codex".into()], &hooks).is_empty());
        assert!(claude_args(&[], &hooks).is_empty());
        assert!(claude_args(&["claude".into(), "--settings".into(), "x.json".into()], &hooks).is_empty());
    }

    #[test]
    fn hooks_are_found_in_the_bundle_and_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Nixa.app/Contents");
        std::fs::create_dir_all(bundle.join("Resources/hooks")).unwrap();
        std::fs::create_dir_all(bundle.join("MacOS")).unwrap();
        std::fs::write(bundle.join("Resources/hooks").join(SCRIPT), "").unwrap();
        let found = hooks_dir(&bundle.join("MacOS/nixa-engine")).unwrap();
        assert!(found.ends_with("Resources/hooks"));
    }
}
