use crate::opts::Opts;
use crate::run;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const BINS: [(&str, &str); 1] = [("nixa-app", "nixa-app")];
pub const LINKS: [(&str, &str); 5] = [("nixa", "cli"), ("nixa-mcp", "mcp"), ("nixa-json", "json"), ("nixa-engine", "engine"), ("nixa-brain", "brain")];

pub fn launcher(word: &str) -> String {
    format!("#!/bin/sh\nme=\"$0\"\n[ -L \"$me\" ] && me=\"$(readlink \"$me\")\"\nexec \"$(cd \"$(dirname \"$me\")\" && pwd)/../../MacOS/{MAIN_BIN}\" {word} \"$@\"\n")
}
pub const MAIN_BIN: &str = "nixa-app";
const FLAG_SEP: char = '\u{1f}';

pub fn release_dir(target_dir: &Path) -> PathBuf {
    target_dir.join("release")
}

pub fn cargo_args() -> Vec<String> {
    let mut args = vec!["build".to_string(), "--release".to_string()];
    for (package, _) in BINS {
        args.push("-p".to_string());
        args.push(package.to_string());
    }
    args
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty() && h != "/")
        .map(PathBuf::from)
}

pub fn rust_flags(home: Option<&Path>, root: &Path, before: &[String]) -> Vec<String> {
    let mut flags = before.to_vec();
    if let Some(home) = home {
        flags.push(format!("--remap-path-prefix={}=~", home.display()));
    }
    flags.push(format!("--remap-path-prefix={}=.", root.display()));
    flags
}

pub fn c_flags(home: Option<&Path>, root: &Path, before: &str) -> String {
    let mut parts: Vec<String> = before.split_whitespace().map(str::to_string).collect();
    if let Some(home) = home {
        parts.push(format!("-ffile-prefix-map={}=~", home.display()));
    }
    parts.push(format!("-ffile-prefix-map={}=.", root.display()));
    parts.join(" ")
}

pub fn old_rust_flags(encoded: Option<String>, plain: Option<String>) -> Vec<String> {
    if let Some(encoded) = encoded.filter(|e| !e.is_empty()) {
        return encoded.split(FLAG_SEP).map(str::to_string).collect();
    }
    plain.unwrap_or_default().split_whitespace().map(str::to_string).collect()
}

pub fn build(opts: &Opts) -> Result<PathBuf, String> {
    let home = home();
    let before = old_rust_flags(std::env::var("CARGO_ENCODED_RUSTFLAGS").ok(), std::env::var("RUSTFLAGS").ok());
    let flags = rust_flags(home.as_deref(), &opts.root, &before);
    let cflags = c_flags(home.as_deref(), &opts.root, &std::env::var("CFLAGS").unwrap_or_default());
    let cxxflags = c_flags(home.as_deref(), &opts.root, &std::env::var("CXXFLAGS").unwrap_or_default());
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(cargo_args())
        .current_dir(&opts.root)
        .env("CARGO_TARGET_DIR", &opts.target_dir)
        .env("CARGO_ENCODED_RUSTFLAGS", flags.join(&FLAG_SEP.to_string()))
        .env_remove("RUSTFLAGS")
        .env("CFLAGS", cflags)
        .env("CXXFLAGS", cxxflags);
    println!("Building the release programs into {}", opts.target_dir.display());
    run::status(&mut cmd)?;
    Ok(release_dir(&opts.target_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_program_is_built_in_release() {
        let args = cargo_args();
        assert_eq!(&args[..2], ["build", "--release"]);
        assert_eq!(args, ["build", "--release", "-p", "nixa-app"]);
        let bins: Vec<&str> = BINS.iter().map(|(_, b)| *b).collect();
        assert_eq!(bins, ["nixa-app"]);
        let names: Vec<&str> = LINKS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["nixa", "nixa-mcp", "nixa-json", "nixa-engine", "nixa-brain"]);
        assert!(launcher("cli").ends_with("nixa-app\" cli \"$@\"\n"));
        assert_eq!(release_dir(Path::new("/t")), Path::new("/t/release"));
    }

    #[test]
    fn build_paths_are_hidden_in_the_programs() {
        let home = Path::new("/Users/someone");
        let root = Path::new("/Users/someone/src/nixa");
        let flags = rust_flags(Some(home), root, &["-Copt-level=3".to_string()]);
        assert_eq!(
            flags,
            ["-Copt-level=3", "--remap-path-prefix=/Users/someone=~", "--remap-path-prefix=/Users/someone/src/nixa=."]
        );
        assert_eq!(rust_flags(None, Path::new("/r"), &[]), ["--remap-path-prefix=/r=."]);
        assert_eq!(c_flags(Some(home), Path::new("/r"), " -O2 "), "-O2 -ffile-prefix-map=/Users/someone=~ -ffile-prefix-map=/r=.");
    }

    #[test]
    fn old_flags_are_kept() {
        assert_eq!(old_rust_flags(Some("-a\u{1f}-b c".into()), Some("-x".into())), ["-a", "-b c"]);
        assert_eq!(old_rust_flags(Some(String::new()), Some(" -x  -y ".into())), ["-x", "-y"]);
        assert!(old_rust_flags(None, None).is_empty());
    }
}
