use super::git;
use super::tree::{self, file_name, parent, safe_relative};
use anyhow::{Result, anyhow, bail};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    Run,
    Gitignore,
    Exclude,
    Edit,
    Terminal,
    NewFile,
    NewFolder,
    Rename,
    Delete,
    Reveal,
    Open,
    Copy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub root: String,
    pub path: String,
    pub dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    Made(String),
    Renamed(String),
    Trashed(String),
    Ignored { pattern: String, already: bool },
}

impl Target {
    pub fn full(&self) -> String {
        tree::join(&self.root, &self.path)
    }
}

impl FileAction {
    pub fn label(self) -> &'static str {
        match self {
            FileAction::Run => "Run",
            FileAction::Gitignore => "Add to .gitignore",
            FileAction::Exclude => "Add to .git/info/exclude",
            FileAction::Edit => "Edit in editor",
            FileAction::Terminal => "Open in terminal",
            FileAction::NewFile => "New file",
            FileAction::NewFolder => "New folder",
            FileAction::Rename => "Rename",
            FileAction::Delete => "Delete",
            FileAction::Reveal => "Reveal in Finder",
            FileAction::Open => "Open with default app",
            FileAction::Copy => "Copy path",
        }
    }

    pub fn asks_name(self) -> bool {
        matches!(
            self,
            FileAction::NewFile | FileAction::NewFolder | FileAction::Rename
        )
    }
}

pub fn menu_items(target: &Target) -> Vec<FileAction> {
    let mut items = Vec::new();
    if !target.dir && runner_for(&target.path).is_some() {
        items.push(FileAction::Run);
    }
    items.extend([
        FileAction::Gitignore,
        FileAction::Exclude,
        FileAction::Edit,
        FileAction::Terminal,
        FileAction::NewFile,
        FileAction::NewFolder,
        FileAction::Rename,
        FileAction::Delete,
        FileAction::Reveal,
        FileAction::Open,
        FileAction::Copy,
    ]);
    if target.dir {
        items.retain(|a| !matches!(a, FileAction::Edit | FileAction::Terminal));
    }
    items
}

pub fn runner_for(path: &str) -> Option<&'static str> {
    let ext = super::tone::extension(file_name(path));
    Some(match ext.as_str() {
        "sh" | "bash" | "command" => "bash",
        "zsh" => "zsh",
        "ksh" => "ksh",
        "fish" => "fish",
        "py" => "python3",
        "rb" => "ruby",
        "pl" => "perl",
        "js" | "mjs" => "node",
        _ => return None,
    })
}

pub fn run_argv(target: &Target) -> Result<(String, String, Vec<String>)> {
    check(target)?;
    let bin = match runner_for(&target.path) {
        Some(bin) if !target.dir => bin,
        _ => bail!("This file cannot be run"),
    };
    let full = target.full();
    let folder = tree::join(&target.root, parent(&target.path));
    Ok((
        file_name(&target.path).to_string(),
        folder,
        vec![bin.to_string(), full],
    ))
}

pub fn safe_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

pub fn check(target: &Target) -> Result<()> {
    if target.root.is_empty() || !safe_relative(&target.path) {
        bail!("File actions must stay inside the shown folder");
    }
    Ok(())
}

pub fn inside_root(root: &str, to: &str) -> Result<()> {
    let root = fs::canonicalize(root).map_err(|e| anyhow!("Cannot read the shown folder: {e}"))?;
    let dir = Path::new(to).parent().ok_or_else(|| anyhow!("File actions must stay inside the shown folder"))?;
    let real = fs::canonicalize(dir).map_err(|e| anyhow!("Cannot read {}: {e}", dir.display()))?;
    if !real.starts_with(&root) {
        bail!("File actions must stay inside the shown folder");
    }
    Ok(())
}

pub fn new_path(target: &Target, name: &str) -> String {
    let base = if target.dir {
        target.path.as_str()
    } else {
        parent(&target.path)
    };
    tree::join(&target.root, &tree::child(base, name))
}

pub fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

pub fn run(action: FileAction, target: &Target, name: &str, home: &Path) -> Result<Done> {
    check(target)?;
    if action.asks_name() && !safe_name(name) {
        bail!("Enter one valid file or folder name");
    }
    let from = PathBuf::from(target.full());
    match action {
        FileAction::NewFile => {
            let to = new_path(target, name);
            inside_root(&target.root, &to)?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&to)
                .map_err(|e| anyhow!("Cannot make {name}: {e}"))?;
            Ok(Done::Made(to))
        }
        FileAction::NewFolder => {
            let to = new_path(target, name);
            inside_root(&target.root, &to)?;
            fs::create_dir(&to).map_err(|e| anyhow!("Cannot make {name}: {e}"))?;
            Ok(Done::Made(to))
        }
        FileAction::Rename => {
            let to = tree::join(&target.root, &tree::child(parent(&target.path), name));
            if to == target.full() {
                return Ok(Done::Renamed(to));
            }
            if exists(Path::new(&to)) {
                bail!("\"{name}\" already exists here");
            }
            inside_root(&target.root, &to)?;
            inside_root(&target.root, &from.display().to_string())?;
            fs::rename(&from, &to).map_err(|e| anyhow!("Cannot rename: {e}"))?;
            Ok(Done::Renamed(to))
        }
        FileAction::Delete => trash(&from, home).map(Done::Trashed),
        FileAction::Gitignore | FileAction::Exclude => ignore(action, target),
        _ => bail!("This action is not run here"),
    }
}

pub fn trash(from: &Path, home: &Path) -> Result<String> {
    if !exists(from) {
        bail!("{} is gone", from.display());
    }
    let bin = home.join(".Trash");
    fs::create_dir_all(&bin)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = from
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "item".into());
    let mut to = bin.join(&name);
    if exists(&to) {
        to = bin.join(format!("{name}-{stamp}"));
    }
    if let Err(error) = fs::rename(from, &to) {
        if error.kind() != std::io::ErrorKind::CrossesDevices {
            bail!("Cannot move it to the Trash: {error}");
        }
        let moved = std::process::Command::new("/bin/mv").arg("-n").arg("--").arg(from).arg(&to).output()?;
        if !moved.status.success() || exists(from) {
            bail!("Cannot move it to the Trash: {}", String::from_utf8_lossy(&moved.stderr).trim());
        }
    }
    Ok(to.display().to_string())
}

pub fn gitignore_pattern(rel: &str, dir: bool) -> Result<String> {
    if rel.is_empty()
        || rel == "."
        || rel.contains(['\0', '\n', '\r'])
        || rel
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        bail!("This path cannot be ignored");
    }
    let mut body = String::new();
    for c in rel.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            body.push('\\');
        }
        body.push(c);
    }
    let kept = body.trim_end_matches(' ').len();
    let spaces = body.len() - kept;
    body.truncate(kept);
    body.push_str(&"\\ ".repeat(spaces));
    Ok(format!("/{body}{}", if dir { "/" } else { "" }))
}

fn ignore(action: FileAction, target: &Target) -> Result<Done> {
    let from = target.full();
    let place = if target.dir {
        from.clone()
    } else {
        tree::join(&target.root, parent(&target.path))
    };
    let where_ = Path::new(&place);
    let prefix = git::git(where_, &["rev-parse", "--show-prefix"])
        .ok_or_else(|| anyhow!("Not a git repository"))?;
    let prefix = prefix.trim().trim_end_matches('/');
    let rel = if target.dir {
        prefix.to_string()
    } else if prefix.is_empty() {
        file_name(&target.path).to_string()
    } else {
        format!("{prefix}/{}", file_name(&target.path))
    };
    if target.dir && rel.is_empty() {
        bail!("Cannot ignore the repository root");
    }
    let pattern = gitignore_pattern(&rel, target.dir)?;
    let dest = if action == FileAction::Exclude {
        let raw = git::git(where_, &["rev-parse", "--git-path", "info/exclude"])
            .ok_or_else(|| anyhow!("Not a git repository"))?;
        let raw = PathBuf::from(raw.trim());
        let dest = if raw.is_absolute() { raw } else { where_.join(raw) };
        let top = git::top(where_).ok_or_else(|| anyhow!("Not a git repository"))?;
        let top = fs::canonicalize(&top)?;
        let info = dest.parent().ok_or_else(|| anyhow!("Not a git repository"))?;
        let git_dir = info.parent().ok_or_else(|| anyhow!("Not a git repository"))?;
        let git_real = fs::canonicalize(git_dir).map_err(|e| anyhow!("Cannot read {}: {e}", git_dir.display()))?;
        let info_real = fs::canonicalize(info).unwrap_or_else(|_| git_real.join("info"));
        let git_ok = git_real.starts_with(&top) || git_real.file_name().is_some_and(|n| n == ".git");
        if !git_ok || !info_real.starts_with(&git_real) {
            bail!("The git folder is outside this repository, so Nixa does not change it");
        }
        dest
    } else {
        git::top(where_)
            .ok_or_else(|| anyhow!("Not a git repository"))?
            .join(".gitignore")
    };
    let already = add_line(&dest, &pattern)?;
    Ok(Done::Ignored { pattern, already })
}

pub fn add_line(dest: &Path, line: &str) -> Result<bool> {
    if let Ok(meta) = dest.symlink_metadata()
        && (meta.file_type().is_symlink() || !meta.is_file())
    {
        bail!(
            "{} is a link or not a plain file, so Nixa does not change it",
            dest.display()
        );
    }
    let old = fs::read_to_string(dest).unwrap_or_default();
    if old.lines().any(|l| l == line) {
        return Ok(true);
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(dest)?;
    if !old.is_empty() && !old.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.write_all(format!("{line}\n").as_bytes())?;
    Ok(false)
}

pub fn done_title(action: FileAction, done: Option<&Done>, target: &Target) -> String {
    match (action, done) {
        (FileAction::Gitignore, Some(Done::Ignored { already: true, .. })) => {
            "Already in .gitignore".into()
        }
        (FileAction::Gitignore, _) => "Added to .gitignore".into(),
        (FileAction::Exclude, Some(Done::Ignored { already: true, .. })) => {
            "Already in .git/info/exclude".into()
        }
        (FileAction::Exclude, _) => "Added to .git/info/exclude".into(),
        (FileAction::NewFile, _) => "File created".into(),
        (FileAction::NewFolder, _) => "Folder created".into(),
        (FileAction::Rename, _) => "Renamed".into(),
        (FileAction::Delete, _) => "Moved to the Trash".into(),
        (FileAction::Reveal, _) => "Revealed in Finder".into(),
        (FileAction::Open, _) => "Opened".into(),
        (FileAction::Copy, _) => "Path copied".into(),
        (FileAction::Run, _) => format!("Running {}", file_name(&target.path)),
        (FileAction::Edit, _) => format!("Editing {}", file_name(&target.path)),
        (FileAction::Terminal, _) => format!("Opened {} in a terminal", file_name(&target.path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nixa-files-act-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn target(root: &Path, path: &str, dir: bool) -> Target {
        Target {
            root: root.display().to_string(),
            path: path.into(),
            dir,
        }
    }

    #[test]
    fn menu_has_run_only_for_scripts() {
        let root = Path::new("/r");
        let script = menu_items(&target(root, "a/run.sh", false));
        assert_eq!(script[0], FileAction::Run);
        assert_eq!(script.len(), 12);
        let plain = menu_items(&target(root, "a/notes.md", false));
        assert!(!plain.contains(&FileAction::Run));
        assert!(plain.contains(&FileAction::Terminal));
        let folder = menu_items(&target(root, "a", true));
        assert!(!folder.contains(&FileAction::Run));
        assert!(!folder.contains(&FileAction::Edit));
        assert!(!folder.contains(&FileAction::Terminal));
    }

    #[test]
    fn runners_by_extension() {
        assert_eq!(runner_for("x/run.sh"), Some("bash"));
        assert_eq!(runner_for("go.command"), Some("bash"));
        assert_eq!(runner_for("tool.py"), Some("python3"));
        assert_eq!(runner_for("app.mjs"), Some("node"));
        assert_eq!(runner_for(".sh"), None);
        assert_eq!(runner_for("Makefile"), None);
        let (name, folder, argv) = run_argv(&target(Path::new("/r"), "bin/go.sh", false)).unwrap();
        assert_eq!(name, "go.sh");
        assert_eq!(folder, "/r/bin");
        assert_eq!(argv, vec!["bash".to_string(), "/r/bin/go.sh".to_string()]);
        assert!(run_argv(&target(Path::new("/r"), "x.md", false)).is_err());
    }

    #[test]
    fn names_and_targets_are_checked() {
        assert!(safe_name("ok.txt"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(!safe_name(bad), "{bad:?}");
        }
        let home = temp("home-check");
        let bad = Target {
            root: "/r".into(),
            path: "../etc".into(),
            dir: true,
        };
        assert!(run(FileAction::NewFile, &bad, "x", &home).is_err());
        let root = temp("names");
        let t = target(&root, "a.txt", false);
        assert!(run(FileAction::NewFile, &t, "../b", &home).is_err());
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn new_rename_and_trash() {
        let root = temp("crud");
        let home = temp("crud-home");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.txt"), "x").unwrap();
        let in_src = target(&root, "src", true);
        let made = run(FileAction::NewFile, &in_src, "b.txt", &home).unwrap();
        assert_eq!(
            made,
            Done::Made(root.join("src/b.txt").display().to_string())
        );
        assert!(run(FileAction::NewFile, &in_src, "b.txt", &home).is_err());
        let beside = target(&root, "src/a.txt", false);
        run(FileAction::NewFolder, &beside, "inner", &home).unwrap();
        assert!(root.join("src/inner").is_dir());
        assert!(
            run(FileAction::Rename, &beside, "b.txt", &home)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        run(FileAction::Rename, &beside, "c.txt", &home).unwrap();
        assert!(root.join("src/c.txt").exists());
        let c = target(&root, "src/c.txt", false);
        let Done::Trashed(to) = run(FileAction::Delete, &c, "", &home).unwrap() else {
            panic!()
        };
        assert!(!root.join("src/c.txt").exists());
        assert!(Path::new(&to).exists());
        assert!(to.starts_with(&home.join(".Trash").display().to_string()));
        fs::write(root.join("src/c.txt"), "again").unwrap();
        let Done::Trashed(second) = run(FileAction::Delete, &c, "", &home).unwrap() else {
            panic!()
        };
        assert_ne!(second, to);
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn ignore_patterns_are_literal() {
        assert_eq!(gitignore_pattern("a/b.txt", false).unwrap(), "/a/b.txt");
        assert_eq!(gitignore_pattern("build", true).unwrap(), "/build/");
        assert_eq!(
            gitignore_pattern("x*?[y].txt", false).unwrap(),
            "/x\\*\\?\\[y].txt"
        );
        assert_eq!(gitignore_pattern("end  ", false).unwrap(), "/end\\ \\ ");
        assert!(gitignore_pattern("", false).is_err());
        assert!(gitignore_pattern("a/../b", false).is_err());
        assert!(gitignore_pattern("a\nb", false).is_err());
    }

    #[test]
    fn new_file_under_an_outside_link_is_refused() {
        let root = temp("outlink");
        let away = temp("outlink-away");
        std::os::unix::fs::symlink(&away, root.join("out")).unwrap();
        let t = target(&root, "out", true);
        let home = temp("outlink-home");
        assert!(run(FileAction::NewFile, &t, "x.txt", &home).is_err());
        assert!(run(FileAction::NewFolder, &t, "d", &home).is_err());
        assert!(!away.join("x.txt").exists());
        fs::create_dir(root.join("in")).unwrap();
        assert!(run(FileAction::NewFile, &target(&root, "in", true), "x.txt", &home).is_ok());
    }

    #[test]
    fn exclude_refuses_a_gitdir_outside_the_repo() {
        let root = temp("gitdir");
        let other = temp("gitdir-other");
        let ok = Command::new("git").arg("init").arg("-q").arg("--bare").arg(&other).status().unwrap();
        assert!(ok.success());
        fs::write(root.join(".git"), format!("gitdir: {}\n", other.display())).unwrap();
        fs::write(root.join("a.txt"), "x").unwrap();
        assert!(run(FileAction::Exclude, &target(&root, "a.txt", false), "", &temp("gitdir-home")).is_err());
        assert!(!fs::read_to_string(other.join("info/exclude")).unwrap_or_default().contains("a.txt"));
    }

    #[test]
    fn ignore_and_exclude_write_once() {
        let root = temp("ignore");
        let home = temp("ignore-home");
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/log.txt"), "x").unwrap();
        fs::write(root.join(".gitignore"), "old").unwrap();
        let t = target(&root, "sub/log.txt", false);
        let first = run(FileAction::Gitignore, &t, "", &home).unwrap();
        assert_eq!(
            first,
            Done::Ignored {
                pattern: "/sub/log.txt".into(),
                already: false
            }
        );
        let again = run(FileAction::Gitignore, &t, "", &home).unwrap();
        assert_eq!(
            again,
            Done::Ignored {
                pattern: "/sub/log.txt".into(),
                already: true
            }
        );
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "old\n/sub/log.txt\n"
        );
        let folder = target(&root, "sub", true);
        run(FileAction::Exclude, &folder, "", &home).unwrap();
        let exclude = fs::read_to_string(root.join(".git/info/exclude")).unwrap();
        assert!(exclude.lines().any(|l| l == "/sub/"), "{exclude}");
        let top = Target {
            root: root.display().to_string(),
            path: "sub".into(),
            dir: true,
        };
        let sub_root = Target {
            root: root.join("sub").display().to_string(),
            path: "x".into(),
            dir: false,
        };
        fs::write(root.join("sub/x"), "").unwrap();
        assert!(run(FileAction::Exclude, &sub_root, "", &home).is_ok());
        assert!(run(FileAction::Exclude, &top, "", &home).is_ok());
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn ignore_files_that_are_links_are_not_written() {
        let dir = temp("link");
        let outside = dir.join("outside.txt");
        fs::write(&outside, "keep\n").unwrap();
        let link = dir.join("ignore-link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err = add_line(&link, "/x").unwrap_err().to_string();
        assert!(err.contains("is a link"), "{err}");
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep\n");
        fs::create_dir_all(dir.join("folder")).unwrap();
        assert!(add_line(&dir.join("folder"), "/x").is_err());
        let plain = dir.join("plain");
        assert!(!add_line(&plain, "/x").unwrap());
        assert!(add_line(&plain, "/x").unwrap());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn titles() {
        let t = target(Path::new("/r"), "go.sh", false);
        assert_eq!(done_title(FileAction::Run, None, &t), "Running go.sh");
        let already = Done::Ignored {
            pattern: "/x".into(),
            already: true,
        };
        assert_eq!(
            done_title(FileAction::Exclude, Some(&already), &t),
            "Already in .git/info/exclude"
        );
    }
}
