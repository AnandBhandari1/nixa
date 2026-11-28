use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Nixa";
pub const BUNDLE_ID: &str = "dev.nixa.app";
pub const AD_HOC: &str = "-";

pub const HELP: &str = "cargo run -p xtask -- <command> [options]

Commands:
  bundle   build the release programs and make target/bundle/Nixa.app
  check    check a made bundle (default target/bundle/Nixa.app, or give its path)
  icon     make crates/xtask/assets/AppIcon.icns again from the Nixa mascot
  soak     run Nixa with fake agents for hours (cargo run -p xtask -- soak help)
  release  inspect a staged bundle or install it from a separate macOS terminal
  help     show this text

Options:
  --out DIR          folder for Nixa.app (default <repo>/target/bundle)
  --target-dir DIR   cargo target folder for the release build (default <repo>/target/gpui)
  --root DIR         repo folder (default: current Nixa workspace, then build-time repo)
  --sign IDENTITY    code signing identity (default AutoNotes Local when present, else -, ad hoc)
  --no-build         use the release programs already in the target folder
  --no-live-check    skip the check that starts the bundled brain and asks the phone door for page routes
  --allow-missing-licenses   make or check a test bundle without THIRD_PARTY_LICENSES.html or LICENSE
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opts {
    pub root: PathBuf,
    pub out: PathBuf,
    pub target_dir: PathBuf,
    pub sign: Option<String>,
    pub build: bool,
    pub live: bool,
    pub licenses: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Bundle(Opts),
    Check { app: PathBuf, root: PathBuf, live: bool, licenses: bool },
    Icon { root: PathBuf },
    Soak(Vec<String>),
    Release(Vec<String>),
    Help,
}

pub fn default_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(root) = workspace_from(&cwd)
    {
        return root.to_path_buf();
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).unwrap_or(manifest).to_path_buf()
}

fn workspace_from(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|root| {
        root.join("Cargo.lock").is_file()
            && root.join("crates/xtask/Cargo.toml").is_file()
            && root.join("crates/app/Cargo.toml").is_file()
    })
}

pub fn app_path(out: &Path) -> PathBuf {
    out.join(format!("{APP_NAME}.app"))
}

fn full(path: &str) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|e| format!("bad path {path}: {e}"))
}

fn value<'a>(rest: &mut impl Iterator<Item = &'a String>, name: &str) -> Result<&'a String, String> {
    rest.next()
        .filter(|v| !v.is_empty() && !v.starts_with("--"))
        .ok_or_else(|| format!("{name} needs a value"))
}

pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let Some((word, rest)) = args.split_first() else {
        return Ok(Cmd::Help);
    };
    if word == "soak" {
        return Ok(Cmd::Soak(rest.to_vec()));
    }
    if word == "release" {
        return Ok(Cmd::Release(rest.to_vec()));
    }
    let mut root = default_root();
    let mut out = None;
    let mut target_dir = None;
    let mut sign = None;
    let mut build = true;
    let mut live = true;
    let mut licenses = true;
    let mut pos = Vec::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => root = full(value(&mut it, arg)?)?,
            "--out" => out = Some(full(value(&mut it, arg)?)?),
            "--target-dir" => target_dir = Some(full(value(&mut it, arg)?)?),
            "--sign" => sign = Some(value(&mut it, arg)?.clone()),
            "--no-build" => build = false,
            "--no-live-check" => live = false,
            "--allow-missing-licenses" => licenses = false,
            "-h" | "--help" => return Ok(Cmd::Help),
            other if other.starts_with('-') => return Err(format!("unknown option {other}. See: cargo run -p xtask -- help")),
            other => pos.push(full(other)?),
        }
    }
    let out = out.unwrap_or_else(|| root.join("target").join("bundle"));
    let target_dir = target_dir.unwrap_or_else(|| root.join("target").join("gpui"));
    match (word.as_str(), pos.as_slice()) {
        ("bundle", []) => Ok(Cmd::Bundle(Opts { root, out, target_dir, sign, build, live, licenses })),
        ("check", []) => Ok(Cmd::Check { app: app_path(&out), root, live, licenses }),
        ("check", [app]) => Ok(Cmd::Check { app: app.clone(), root, live, licenses }),
        ("icon", []) => Ok(Cmd::Icon { root }),
        ("help" | "-h" | "--help", _) => Ok(Cmd::Help),
        ("bundle" | "check" | "icon", extra) => Err(format!("{word} does not take {}", extra[extra.len() - 1].display())),
        (other, _) => Err(format!("unknown command {other}. See: cargo run -p xtask -- help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bundle_defaults_live_in_the_repo_target_folder() {
        let root = default_root();
        assert!(root.join("crates").join("xtask").join("Cargo.toml").is_file());
        let Cmd::Bundle(opts) = parse(&words(&["bundle"])).unwrap() else { panic!("not bundle") };
        assert_eq!(opts.out, root.join("target/bundle"));
        assert_eq!(opts.target_dir, root.join("target/gpui"));
        assert_eq!(opts.sign, None);
        assert!(opts.build && opts.live && opts.licenses);
        assert_eq!(app_path(&opts.out), root.join("target/bundle/Nixa.app"));
    }

    #[test]
    fn options_are_read() {
        let cmd = parse(&words(&["bundle", "--out", "/tmp/o", "--target-dir", "/tmp/t", "--root", "/tmp/r", "--sign", "Nixa Local", "--no-build", "--no-live-check", "--allow-missing-licenses"])).unwrap();
        assert_eq!(
            cmd,
            Cmd::Bundle(Opts {
                root: PathBuf::from("/tmp/r"),
                out: PathBuf::from("/tmp/o"),
                target_dir: PathBuf::from("/tmp/t"),
                sign: Some("Nixa Local".into()),
                build: false,
                live: false,
                licenses: false,
            })
        );
        let cmd = parse(&words(&["check", "--root", "/tmp/r"])).unwrap();
        assert_eq!(cmd, Cmd::Check { app: PathBuf::from("/tmp/r/target/bundle/Nixa.app"), root: PathBuf::from("/tmp/r"), live: true, licenses: true });
        let cmd = parse(&words(&["check", "/tmp/x/A.app", "--no-live-check", "--allow-missing-licenses"])).unwrap();
        assert!(matches!(cmd, Cmd::Check { app, live: false, licenses: false, .. } if app == Path::new("/tmp/x/A.app")));
    }

    #[test]
    fn finds_workspace_from_nested_folder_after_a_move() {
        let moved = crate::testkit::temp("moved-root");
        let nested = moved.join("crates/app/src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(moved.join("crates/xtask")).unwrap();
        for file in ["Cargo.lock", "crates/app/Cargo.toml", "crates/xtask/Cargo.toml"] {
            std::fs::write(moved.join(file), b"").unwrap();
        }
        assert_eq!(workspace_from(&nested), Some(moved.as_path()));
        assert_eq!(workspace_from(moved.parent().unwrap()), None);
        std::fs::remove_dir_all(moved).unwrap();
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(parse(&[]).unwrap(), Cmd::Help);
        assert_eq!(parse(&words(&["help"])).unwrap(), Cmd::Help);
        assert_eq!(parse(&words(&["soak", "--for", "4h", "--agents", "6"])).unwrap(), Cmd::Soak(words(&["--for", "4h", "--agents", "6"])));
        assert_eq!(parse(&words(&["soak"])).unwrap(), Cmd::Soak(Vec::new()));
        assert!(parse(&words(&["bundel"])).unwrap_err().contains("unknown command bundel"));
        assert!(parse(&words(&["bundle", "--fast"])).unwrap_err().contains("unknown option --fast"));
        assert!(parse(&words(&["bundle", "--out"])).unwrap_err().contains("--out needs a value"));
        assert!(parse(&words(&["bundle", "--sign", "--no-build"])).unwrap_err().contains("--sign needs a value"));
        assert!(parse(&words(&["bundle", "/tmp/x"])).unwrap_err().contains("bundle does not take /tmp/x"));
        assert!(parse(&words(&["check", "/a", "/b"])).is_err());
    }
}
