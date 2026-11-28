use super::call;
use super::secrets::{self, Keychain};
use super::store;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub const USAGE: &str = "nixa connector: outside tools for Nixa's brain

Connectors live in the connectors folder (nixa connector folder), one folder each:
connector-<name>/connector.json. Secrets are kept in the macOS keychain
(service nixa-connector) and are never shown.

  list                      installed connectors, their status and secret entries (no values)
  secret ENTRY              type or paste the secret for one entry; it is hidden and kept only in the keychain
  secret ENTRY --remove     remove that secret from the keychain
  trust NAME                kept for old scripts; every step already runs without asking
  untrust NAME              kept for old scripts; it changes nothing
  log [NAME] [--limit N]    the last calls, oldest first (default 20)
  folder                    print the connectors folder
";

pub fn root() -> std::path::PathBuf {
    nixa_protocol::paths::data_dir().join(super::FOLDER)
}

pub fn main(args: &[String]) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("nixa connector: {e}");
            return 1;
        }
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let code = runtime.block_on(run(args, &root(), &secrets::system(), &mut read_hidden, &mut out));
    let _ = out.flush();
    code
}

fn stty(arg: &str) -> bool {
    Command::new("/bin/stty").arg(arg).stdin(Stdio::inherit()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

pub fn read_hidden(prompt: &str) -> String {
    let stdin = std::io::stdin();
    let mut line = String::new();
    if !stdin.is_terminal() {
        let _ = stdin.lock().take(64 * 1024).read_to_string(&mut line);
        return line;
    }
    eprint!("{prompt}");
    let quiet = stty("-echo");
    let _ = stdin.lock().read_line(&mut line);
    if quiet {
        stty("echo");
    }
    eprintln!();
    line
}

pub fn iso(ms: u64) -> String {
    let (y, m, d) = nixa_store::time::civil_from_days((ms / 86_400_000) as i64);
    let rest = ms % 86_400_000;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60)
}

fn say(out: &mut dyn Write, line: &str) {
    let _ = writeln!(out, "{line}");
}

async fn list(root: &Path, kc: &Keychain, out: &mut dyn Write) -> i32 {
    let installed = call::scan(root);
    if installed.is_empty() {
        say(out, &format!("No connectors installed. Folder: {}", super::folder_text(root)));
    }
    for item in installed {
        let manifest = match item.manifest {
            Ok(m) => m,
            Err(e) => {
                say(out, &format!("{}: broken: {e}", item.name));
                continue;
            }
        };
        let actions: Vec<String> = manifest.actions.iter().map(|a| format!("{} ({}, {})", a.name, a.permission.word(), a.reach.word())).collect();
        let trusted = if store::trusted(root, &manifest.connector) { " [trusted]" } else { "" };
        say(out, &format!("{} ({}): {}{trusted}", manifest.connector, manifest.kind.word(), actions.join(", ")));
        let set = call::secrets_of(kc, &manifest).await;
        for entry in manifest.all_secrets() {
            let state = if set.contains_key(&entry) { "set" } else { "missing" };
            let optional = if manifest.secrets.contains(&entry) { "" } else { " (optional)" };
            say(out, &format!("  secret {entry}: {state}{optional}"));
        }
    }
    0
}

fn limit_of(args: &[String]) -> Result<(Option<String>, usize), String> {
    let mut name = None;
    let mut limit = super::DEFAULT_CALLS;
    let mut i = 0;
    while let Some(a) = args.get(i) {
        if a == "--limit" {
            limit = args.get(i + 1).and_then(|v| v.parse().ok()).filter(|n| *n >= 1).ok_or("--limit needs a number")?;
            i += 2;
        } else if name.is_none() && !a.starts_with('-') {
            name = Some(a.clone());
            i += 1;
        } else {
            return Err(format!("unknown option {a}"));
        }
    }
    Ok((name, limit.min(super::MAX_CALLS)))
}

fn log(root: &Path, args: &[String], out: &mut dyn Write) -> i32 {
    let (name, limit) = match limit_of(args) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("nixa connector: {e}");
            return 2;
        }
    };
    let mut rows = store::calls(root, limit, name.as_deref());
    if rows.is_empty() {
        say(out, "No calls logged.");
    }
    rows.reverse();
    for row in rows {
        say(out, &format!("{} {}.{} {}/{} {} by {} {}ms input {} result {}", iso(row.at), row.connector, row.action, row.permission, row.reach, row.verdict, row.approved_by, row.ms, row.input, crate::util::clip_flat(&row.result, 120)));
    }
    0
}

pub async fn run(args: &[String], root: &Path, kc: &Keychain, read: &mut dyn FnMut(&str) -> String, out: &mut dyn Write) -> i32 {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let done = match words.as_slice() {
        ["list"] => return list(root, kc, out).await,
        ["log", ..] => return log(root, &args[1..], out),
        ["folder"] => {
            say(out, &root.display().to_string());
            return 0;
        }
        ["secret", entry, "--remove"] => call::remove_secret(root, kc, entry).await,
        ["secret", entry] => match call::resolve(root, entry) {
            Err(e) => Err(e),
            Ok(_) => {
                let value = read(&format!("Secret for {entry} (hidden): "));
                call::set_secret(root, kc, entry, value.trim()).await
            }
        },
        ["trust", name] => call::set_trust(root, name, true, crate::util::now_ms()),
        ["untrust", name] => call::set_trust(root, name, false, crate::util::now_ms()),
        [] | ["help"] | ["-h"] | ["--help"] => {
            let _ = out.write_all(USAGE.as_bytes());
            return 0;
        }
        _ => {
            eprintln!("nixa connector: unknown use. Run: nixa connector help");
            return 2;
        }
    };
    match done {
        Ok(message) => {
            say(out, &message);
            0
        }
        Err(e) => {
            eprintln!("nixa connector: {e}");
            1
        }
    }
}
