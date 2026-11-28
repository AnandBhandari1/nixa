use super::run::{self, Output};
use super::{Account, Env, bin_dir, write_private};
use regex::Regex;
use std::path::{Path, PathBuf};

pub const HELPER_NAME: &str = "git-credential-nixa";

pub const TOKEN_PREFIXES: &[&str] = &["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-"];

pub fn helper_path(env: &Env) -> PathBuf {
    bin_dir(env).join(HELPER_NAME)
}

pub fn defaults_path(env: &Env) -> PathBuf {
    bin_dir(env).join("defaults")
}

pub fn looks_like_token(user: &str) -> bool {
    let u = user.trim();
    if u.is_empty() {
        return false;
    }
    let low = u.to_lowercase();
    if TOKEN_PREFIXES.iter().any(|p| low.starts_with(p)) {
        return true;
    }
    u.len() >= 40 && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = [bytes[i + 1], bytes[i + 2]];
            let v = u8::from_str_radix(std::str::from_utf8(&hex).unwrap_or("00"), 16).unwrap_or(0);
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

pub fn mask_url(url: &str) -> String {
    let userinfo = Regex::new(r"(?i)^([a-z][a-z0-9+.-]*://)([^/?#]*)@").expect("userinfo pattern");
    let masked = userinfo
        .replace(url, |c: &regex::Captures| {
            let scheme = &c[1];
            let info = &c[2];
            if info.contains('@') {
                return format!("{scheme}***@");
            }
            let user = info.split(':').next().unwrap_or("");
            if looks_like_token(&percent_decode(user)) {
                return format!("{scheme}***@");
            }
            if info.contains(':') {
                format!("{scheme}{user}:***@")
            } else {
                c[0].to_string()
            }
        })
        .into_owned();
    Regex::new(r"(?i)([?&#](?:access_)?token=)[^&#]*")
        .expect("token query pattern")
        .replace_all(&masked, "${1}***")
        .into_owned()
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn helper_script(service: &str, security: &Path) -> String {
    let prefixes: Vec<String> = TOKEN_PREFIXES.iter().map(|p| format!("{p}*")).collect();
    let mut lines: Vec<String> = [
        "#!/bin/sh",
        "[ \"$1\" = get ] || exit 0",
        "proto= host= user=",
        "while IFS= read -r line; do",
        "  [ -z \"$line\" ] && break",
        "  case \"$line\" in",
        "    protocol=*) proto=${line#protocol=} ;;",
        "    host=*) host=${line#host=} ;;",
        "    username=*) user=${line#username=} ;;",
        "  esac",
        "done",
        "[ \"$proto\" = https ] || exit 0",
        "[ -n \"$host\" ] || exit 0",
        "case \"$host\" in *[!A-Za-z0-9.:-]*) exit 0 ;; esac",
        "token_like() {",
        "  case \"$1\" in",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    lines.push(format!("    {}) return 0 ;;", prefixes.join("|")));
    lines.extend(
        [
            "  esac",
            "  [ ${#1} -ge 40 ] && case \"$1\" in *[!A-Za-z0-9_-]*) return 1 ;; *) return 0 ;; esac",
            "  return 1",
            "}",
            "if [ -n \"$user\" ] && token_like \"$user\"; then",
            "  user=$(git config --get \"credential.https://$host.username\" 2>/dev/null)",
            "  token_like \"$user\" && user=",
            "fi",
            "dir=$(dirname \"$0\")",
            "if [ -z \"$user\" ] && [ -f \"$dir/defaults\" ]; then",
            "  user=$(awk -F= -v h=\"$host\" '$1==h { print substr($0, length(h) + 2); exit }' \"$dir/defaults\")",
            "fi",
            "[ -n \"$user\" ] || exit 0",
            "case \"$user\" in *[!A-Za-z0-9._-]*) exit 0 ;; esac",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    lines.push(format!(
        "tok=$({} find-generic-password -s {} -a \"acct:$host:$user\" -w 2>/dev/null)",
        quote(&security.display().to_string()),
        quote(service)
    ));
    lines.push("[ -n \"$tok\" ] || exit 0".into());
    lines.push("printf 'username=%s\\npassword=%s\\n' \"$user\" \"$tok\"".into());
    lines.push(String::new());
    lines.join("\n")
}

pub fn ensure_helper(env: &Env) -> Result<PathBuf, String> {
    let path = helper_path(env);
    let script = helper_script(&env.service, &env.security);
    write_private(&path, &script, 0o700)?;
    Ok(path)
}

pub fn write_defaults(env: &Env, accounts: &[Account]) -> Result<(), String> {
    let lines: Vec<String> = accounts
        .iter()
        .filter(|a| a.default)
        .map(|a| format!("{}={}", a.host, a.username))
        .collect();
    let text = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    write_private(&defaults_path(env), &text, 0o600)
}

pub fn credential_url(host: &str) -> String {
    format!("https://{host}")
}

pub struct Step {
    pub args: Vec<String>,
    pub ok_codes: &'static [i32],
}

fn step(args: &[&str], ok_codes: &'static [i32]) -> Step {
    Step {
        args: run::strings(args),
        ok_codes,
    }
}

pub fn helper_value(helper: &Path) -> String {
    format!("!'{}'", helper.display().to_string().replace('\'', "'\\''"))
}

pub fn bind_commands(host: &str, username: &str, helper: &Path) -> Vec<Step> {
    let base = format!("credential.{}", credential_url(host));
    let helper_key = format!("{base}.helper");
    let user_key = format!("{base}.username");
    let helper = helper_value(helper);
    vec![
        step(&["config", "--local", "--unset-all", &helper_key], &[0, 5]),
        step(&["config", "--local", "--add", &helper_key, ""], &[0]),
        step(&["config", "--local", "--add", &helper_key, &helper], &[0]),
        step(&["config", "--local", &user_key, username], &[0]),
    ]
}

pub fn unbind_commands(host: &str) -> Vec<Step> {
    let base = format!("credential.{}", credential_url(host));
    let helper_key = format!("{base}.helper");
    let user_key = format!("{base}.username");
    vec![
        step(&["config", "--local", "--unset-all", &helper_key], &[0, 5]),
        step(&["config", "--local", "--unset-all", &user_key], &[0, 5]),
    ]
}

pub fn git(env: &Env, root: &Path, args: &[String]) -> Output {
    let mut all = env.git_env.clone();
    all.extend(run::GIT_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    run::run(
        &env.git,
        args,
        &run::Opts {
            cwd: Some(root.to_path_buf()),
            env: all,
            timeout: Some(std::time::Duration::from_secs(120)),
            ..run::Opts::default()
        },
    )
}

pub fn git_str(env: &Env, root: &Path, args: &[&str]) -> Output {
    git(env, root, &run::strings(args))
}

pub fn git_text(env: &Env, root: &Path, args: &[&str]) -> String {
    let out = git_str(env, root, args);
    if out.ok() { out.stdout.trim().to_string() } else { String::new() }
}

pub fn git_ok(env: &Env, root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_str(env, root, args);
    if out.ok() {
        Ok(out.stdout)
    } else {
        Err(fail_text(&out, args))
    }
}

pub fn fail_text(out: &Output, args: &[&str]) -> String {
    let said = run::tail(&out.said(), 3);
    if said.is_empty() {
        format!("git {} failed", args.join(" "))
    } else {
        said
    }
}

fn run_all(env: &Env, root: &Path, steps: Vec<Step>) -> Result<(), String> {
    for s in steps {
        let out = git(env, root, &s.args);
        if !out.code.is_some_and(|c| s.ok_codes.contains(&c)) {
            let words: Vec<&str> = s.args.iter().map(String::as_str).collect();
            return Err(fail_text(&out, &words));
        }
    }
    Ok(())
}

pub fn bind_repo(env: &Env, root: &Path, host: &str, username: &str) -> Result<PathBuf, String> {
    let helper = ensure_helper(env)?;
    run_all(env, root, bind_commands(host, username, &helper))?;
    Ok(helper)
}

pub fn unbind_repo(env: &Env, root: &Path, host: &str) -> Result<(), String> {
    run_all(env, root, unbind_commands(host))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub username: Option<String>,
    pub helper_installed: bool,
}

pub fn read_binding(env: &Env, root: &Path, host: &str) -> Binding {
    let base = format!("credential.{}", credential_url(host));
    let helpers = git_str(env, root, &["config", "--local", "--get-all", &format!("{base}.helper")]);
    let user = git_str(env, root, &["config", "--local", "--get", &format!("{base}.username")]);
    let list: Vec<&str> = if helpers.ok() {
        let mut l: Vec<&str> = helpers.stdout.split('\n').collect();
        if l.last() == Some(&"") {
            l.pop();
        }
        l
    } else {
        Vec::new()
    };
    let helper = helper_value(&helper_path(env));
    let at = list.iter().position(|h| *h == helper);
    let reset = list.iter().position(|h| h.is_empty());
    Binding {
        username: user.ok().then(|| user.stdout.trim().to_string()),
        helper_installed: matches!((reset, at), (Some(r), Some(a)) if r < a),
    }
}

pub fn host_ok(host: &str) -> bool {
    crate::keychain::valid_host(host)
}

pub fn account_args(host: &str, username: &str, helper: &Path) -> Result<Vec<String>, String> {
    let user_ok = !username.is_empty()
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    let path = helper.display().to_string();
    if !host_ok(host) || !user_ok || path.is_empty() || path.contains(['\n', '\0']) {
        return Err("cannot build the account settings for git".into());
    }
    let key = format!("credential.https://{host}");
    Ok(vec![
        "-c".into(),
        format!("{key}.helper="),
        "-c".into(),
        format!("{key}.helper={}", helper_value(helper)),
        "-c".into(),
        format!("{key}.username={username}"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testkit;
    use crate::keychain;
    use std::process::{Command, Stdio};
    use std::io::Write;

    #[test]
    fn token_like_names_and_masking() {
        assert!(looks_like_token("ghp_abc"));
        assert!(looks_like_token(&"a".repeat(40)));
        assert!(!looks_like_token("ana"));
        assert_eq!(mask_url("https://ghp_x@github.com/a/b"), "https://***@github.com/a/b");
        assert_eq!(mask_url("https://ana:pw@github.com/a/b"), "https://ana:***@github.com/a/b");
        assert_eq!(mask_url("https://ana@github.com/a/b"), "https://ana@github.com/a/b");
        assert_eq!(mask_url("https://x.org/a?token=abc&b=1"), "https://x.org/a?token=***&b=1");
        assert_eq!(percent_decode("a%20b"), "a b");
    }

    #[test]
    fn helper_gives_the_keychain_token_to_git() {
        let kit = testkit::kit();
        let env = &kit.env;
        keychain::save_token(env, "github.com", "ana", "tok_secret").unwrap();
        let helper = ensure_helper(env).unwrap();
        write_defaults(
            env,
            &[Account {
                host: "github.com".into(),
                username: "ana".into(),
                default: true,
                ..Account::default()
            }],
        )
        .unwrap();
        let ask = |input: &str| {
            let mut child = Command::new(&helper)
                .arg("get")
                .envs(env.git_env.iter().cloned())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
        };
        assert_eq!(ask("protocol=https\nhost=github.com\n\n"), "username=ana\npassword=tok_secret\n");
        assert_eq!(ask("protocol=https\nhost=github.com\nusername=ghp_x\n\n"), "username=ana\npassword=tok_secret\n");
        assert_eq!(ask("protocol=http\nhost=github.com\n\n"), "");
        assert_eq!(ask("protocol=https\nhost=other.org\n\n"), "");
    }

    #[test]
    fn helper_script_never_asks_gh() {
        let script = helper_script("svc", Path::new("/usr/bin/security"));
        assert!(!script.contains("auth token"), "{script}");
        let kit = testkit::kit();
        kit.gh_token("ana", "ghtok_fromgh");
        let helper = ensure_helper(&kit.env).unwrap();
        let written = std::fs::read_to_string(&helper).unwrap();
        let gh = kit.env.gh.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        assert!(gh.is_empty() || !written.contains(&gh), "{written}");
        assert!(!written.contains("auth token"));
        let mut child = Command::new(&helper)
            .arg("get")
            .envs(kit.env.git_env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"protocol=https\nhost=github.com\nusername=ana\n\n").unwrap();
        assert_eq!(String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap(), "");
    }

    #[test]
    fn helper_never_runs_text_from_the_host_or_user() {
        let kit = testkit::kit();
        let env = &kit.env;
        keychain::save_token(env, "github.com", "ana", "tok_secret").unwrap();
        let helper = ensure_helper(env).unwrap();
        let marker = helper.with_file_name("ran");
        let m = marker.display().to_string();
        let ask = |input: String| {
            let mut child = Command::new(&helper)
                .arg("get")
                .envs(env.git_env.iter().cloned())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
        };
        let bad = [
            format!("$(touch {m})"),
            format!("`touch {m}`"),
            format!("x\"$(touch {m})\""),
            format!("x';touch {m};'"),
        ];
        for text in &bad {
            assert_eq!(ask(format!("protocol=https\nhost={text}\nusername=ana\n\n")), "");
            assert_eq!(ask(format!("protocol=https\nhost=github.com\nusername={text}\n\n")), "");
        }
        assert!(!marker.exists());
        assert_eq!(
            ask("protocol=https\nhost=github.com\nusername=ana\n\n".into()),
            "username=ana\npassword=tok_secret\n"
        );
    }

    #[test]
    fn bind_and_unbind_a_repo() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("r");
        let helper = bind_repo(env, &repo, "github.com", "ana").unwrap();
        let b = read_binding(env, &repo, "github.com");
        assert_eq!(b.username.as_deref(), Some("ana"));
        assert!(b.helper_installed);
        assert_eq!(
            kit.git(&repo, &["config", "--local", "-z", "--get-all", "credential.https://github.com.helper"]),
            format!("\0{}\0", helper_value(&helper))
        );
        unbind_repo(env, &repo, "github.com").unwrap();
        let b = read_binding(env, &repo, "github.com");
        assert_eq!(b.username, None);
        assert!(!b.helper_installed);
        unbind_repo(env, &repo, "github.com").unwrap();
    }

    #[test]
    fn bind_commands_quote_a_helper_path_with_a_space() {
        let steps = bind_commands("github.com", "ana", Path::new("/a b/helper"));
        assert_eq!(steps[2].args.last().map(String::as_str), Some("!'/a b/helper'"));
        let args = account_args("github.com", "ana", Path::new("/a b/h")).unwrap();
        assert_eq!(args[3], "credential.https://github.com.helper=!'/a b/h'");
    }

    #[test]
    fn account_args_refuse_bad_parts() {
        let args = account_args("github.com", "ana", Path::new("/x/h")).unwrap();
        assert_eq!(args[1], "credential.https://github.com.helper=");
        assert!(account_args("git hub", "ana", Path::new("/x/h")).is_err());
        assert!(account_args("github.com", "a;b", Path::new("/x/h")).is_err());
    }
}
