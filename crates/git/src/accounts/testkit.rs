use crate::accounts::Env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct Kit {
    pub dir: TempDir,
    pub env: Env,
}

const SECURITY: &str = r#"#!/bin/sh
d=$(dirname "$0")/..
store="$d/keychain"
touch "$store"
printf '%s\n' "$*" >> "$d/security.log"
field() { printf '%s' "$1" | sed -n "s/.* $2 \"\([^\"]*\)\".*/\1/p"; }
arg() { want=$1; shift; while [ $# -gt 0 ]; do if [ "$1" = "$want" ]; then printf '%s' "$2"; return; fi; shift; done; }
case "$1" in
  -i)
    while IFS= read -r line; do
      case "$line" in
        add-generic-password*)
          s=$(field "$line" -s); a=$(field "$line" -a); w=$(field "$line" -w)
          grep -v "^$s	$a	" "$store" > "$store.new"; mv "$store.new" "$store"
          printf '%s\t%s\t%s\n' "$s" "$a" "$w" >> "$store" ;;
      esac
    done
    exit 0 ;;
  find-generic-password)
    s=$(arg -s "$@"); a=$(arg -a "$@")
    hit=$(grep "^$s	$a	" "$store" | head -n 1)
    [ -n "$hit" ] || exit 44
    case " $* " in *" -w "*) printf '%s\n' "$hit" | cut -f3 ;; *) echo "keychain item" ;; esac
    exit 0 ;;
  delete-generic-password)
    s=$(arg -s "$@"); a=$(arg -a "$@")
    grep -q "^$s	$a	" "$store" || exit 44
    grep -v "^$s	$a	" "$store" > "$store.new"; mv "$store.new" "$store"
    exit 0 ;;
esac
exit 1
"#;

const GH: &str = r#"#!/bin/sh
d=$(dirname "$0")/../gh
mkdir -p "$d"
if [ "$1 $2" = "auth token" ]; then
  u=; while [ $# -gt 0 ]; do [ "$1" = -u ] && u=$2; shift; done
  [ -f "$d/token-$u" ] || { echo "no oauth token found for $u" >&2; exit 1; }
  cat "$d/token-$u"; exit 0
fi
if [ "$1 $2" = "auth status" ]; then
  [ -f "$d/status.json" ] || exit 1
  cat "$d/status.json"; exit 0
fi
printf '%s|GH_TOKEN=%s|GH_HOST=%s|PWD=%s\n' "$*" "$GH_TOKEN" "$GH_HOST" "$(pwd)" >> "$d/calls.log"
for a in "$@"; do case "$a" in --body-file=*) cat "${a#--body-file=}" > "$d/body.seen" ;; esac; done
key="$1-$2"
[ -f "$d/err-$key" ] && cat "$d/err-$key" >&2
[ -f "$d/reply-$key" ] && cat "$d/reply-$key"
[ -f "$d/code-$key" ] && exit "$(cat "$d/code-$key")"
exit 0
"#;

const CURL: &str = r#"#!/bin/sh
d=$(dirname "$0")/../curl
mkdir -p "$d"
header=$(cat)
tok=$(printf '%s' "$header" | sed -n 's/^Authorization: Bearer //p')
for url; do :; done
printf '%s\n' "$url" >> "$d/urls.log"
printf '%s\n' "$*" >> "$d/args.log"
case "$url" in *"/user/repos"*) f="$d/$tok.repos" ;; *) f="$d/$tok" ;; esac
if [ -f "$f" ]; then cat "$f"; exit 0; fi
echo "curl: (7) Failed to connect to host" >&2
exit 7
"#;

fn script(path: &Path, body: &str) {
    fs::write(path, body).expect("write fake tool");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod fake tool");
}

pub fn kit() -> Kit {
    let dir = tempfile::Builder::new()
        .prefix("mg")
        .tempdir_in("/tmp")
        .expect("temp dir");
    let root = dir.path().to_path_buf();
    let bin = root.join("bin");
    let home = root.join("home");
    fs::create_dir_all(&bin).expect("bin");
    fs::create_dir_all(&home).expect("home");
    fs::write(home.join(".gitconfig"), "").expect("gitconfig");
    script(&bin.join("security"), SECURITY);
    script(&bin.join("gh"), GH);
    script(&bin.join("curl"), CURL);
    let env = Env {
        dir: root.join("data").join("git"),
        home: home.clone(),
        service: "Nixa Test".into(),
        security: bin.join("security"),
        curl: bin.join("curl"),
        git: PathBuf::from("git"),
        gh: Some(bin.join("gh")),
        git_env: vec![
            ("HOME".into(), home.display().to_string()),
            ("GIT_CONFIG_GLOBAL".into(), home.join(".gitconfig").display().to_string()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ],
    };
    Kit { dir, env }
}

impl Kit {
    pub fn root(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    pub fn security_log(&self) -> String {
        fs::read_to_string(self.root().join("security.log")).unwrap_or_default()
    }

    pub fn gh_token(&self, user: &str, token: &str) {
        let d = self.root().join("gh");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(format!("token-{user}")), format!("{token}\n")).unwrap();
    }

    pub fn gh_file(&self, name: &str, body: &str) {
        let d = self.root().join("gh");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(name), body).unwrap();
    }

    pub fn gh_calls(&self) -> String {
        fs::read_to_string(self.root().join("gh").join("calls.log")).unwrap_or_default()
    }

    pub fn curl_reply(&self, name: &str, body: &str) {
        let d = self.root().join("curl");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(name), body).unwrap();
    }

    pub fn curl_args(&self) -> String {
        fs::read_to_string(self.root().join("curl").join("args.log")).unwrap_or_default()
    }

    pub fn repo(&self, name: &str) -> PathBuf {
        let path = self.root().join(name);
        fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q", "-b", "main"]);
        path
    }

    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .envs(self.env.git_env.iter().cloned())
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

pub fn user_reply(login: &str, scopes: &str, expiry: &str) -> String {
    let mut head = String::from("HTTP/2 200\r\ncontent-type: application/json\r\n");
    if !scopes.is_empty() {
        head.push_str(&format!("x-oauth-scopes: {scopes}\r\n"));
    }
    if !expiry.is_empty() {
        head.push_str(&format!("github-authentication-token-expiration: {expiry}\r\n"));
    }
    format!("{head}\r\n{{\"login\":\"{login}\",\"name\":\"Name {login}\",\"avatar_url\":\"https://a/{login}\"}}")
}
