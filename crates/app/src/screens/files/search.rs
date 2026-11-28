use super::tree::safe_relative;
use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const SEARCH_TOTAL: usize = 500;
pub const NAME_TOTAL: usize = 100;
pub const WALK_LIMIT: usize = 20000;
const PER_FILE: &str = "5";
const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "coverage",
    "vendor",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub line: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub names: Vec<Hit>,
    pub lines: Vec<Hit>,
    pub cut: bool,
}

pub fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .map(PathBuf::from)
        .chain(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub fn parse_lines(text: &str, root: &str) -> Vec<Hit> {
    let prefix = format!("{}/", root.trim_end_matches('/'));
    let mut hits = Vec::new();
    for row in text.lines() {
        let Some((full, rest)) = row.split_once('\0') else {
            continue;
        };
        let Some((num, line)) = rest.split_once(':') else {
            continue;
        };
        let Ok(num) = num.parse::<u32>() else {
            continue;
        };
        let path = full.strip_prefix(&prefix).unwrap_or(full);
        if !safe_relative(path) {
            continue;
        }
        hits.push(Hit {
            path: path.to_string(),
            line: Some(num),
            text: line.to_string(),
        });
        if hits.len() >= SEARCH_TOTAL {
            break;
        }
    }
    hits
}

fn run(cmd: &mut Command) -> Result<(i32, String, String)> {
    let out = cmd.stdin(Stdio::null()).output()?;
    Ok((
        out.status.code().unwrap_or(2),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub fn content(root: &str, query: &str) -> Result<Vec<Hit>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let (code, out, err) = match find_tool("rg") {
        Some(rg) => run(Command::new(rg)
            .args([
                "--null",
                "--line-number",
                "--max-count",
                PER_FILE,
                "--fixed-strings",
                "--no-messages",
            ])
            .arg(format!("--regexp={query}"))
            .arg("--")
            .arg(root))?,
        None => run(Command::new("grep")
            .args(["-rnIF", "--null", "-m", PER_FILE])
            .args(SKIP.iter().map(|d| format!("--exclude-dir={d}")))
            .arg("-e")
            .arg(query)
            .arg("--")
            .arg(root))?,
    };
    if code > 1 && out.is_empty() {
        let why = err.trim();
        bail!("{}", if why.is_empty() { "Search failed" } else { why });
    }
    Ok(parse_lines(&out, root))
}

pub fn listed(root: &str) -> Option<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.split('\0')
            .filter(|p| !p.is_empty())
            .take(WALK_LIMIT)
            .map(str::to_string)
            .collect(),
    )
}

pub fn walked(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(items) = fs::read_dir(&dir) else {
            continue;
        };
        for item in items.flatten() {
            if out.len() >= WALK_LIMIT {
                return out;
            }
            let name = item.file_name().to_string_lossy().to_string();
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if !SKIP.contains(&name.as_str()) {
                    stack.push((item.path(), path));
                }
            } else {
                out.push(path);
            }
        }
    }
    out
}

pub fn names(root: &str, query: &str) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let all = listed(root).unwrap_or_else(|| walked(Path::new(root)));
    let mut hits: Vec<Hit> = all
        .into_iter()
        .filter(|p| safe_relative(p))
        .filter(|p| super::tree::file_name(p).to_lowercase().contains(&needle))
        .map(|p| Hit {
            path: p,
            line: None,
            text: String::new(),
        })
        .collect();
    hits.sort_by_key(|h| {
        let name = super::tree::file_name(&h.path).to_lowercase();
        (
            name != needle,
            !name.starts_with(&needle),
            h.path.matches('/').count(),
            h.path.clone(),
        )
    });
    hits.truncate(NAME_TOTAL + 1);
    hits
}

pub fn search(root: &str, query: &str) -> Result<Found> {
    let mut names = names(root, query);
    let lines = content(root, query)?;
    let cut = lines.len() >= SEARCH_TOTAL || names.len() > NAME_TOTAL;
    names.truncate(NAME_TOTAL);
    Ok(Found { names, lines, cut })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nixa-files-find-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_tool_lines() {
        let text =
            "/r/src/a.rs\x0012:fn main() {}\n/r/b.txt\x003:x: y\n/elsewhere/c\x001:z\nbad line\n";
        let hits = parse_lines(text, "/r");
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0],
            Hit {
                path: "src/a.rs".into(),
                line: Some(12),
                text: "fn main() {}".into()
            }
        );
        assert_eq!(hits[1].text, "x: y");
    }

    #[test]
    fn finds_names_and_contents() {
        let dir = temp("both");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("src/widget.rs"), "struct Gadget;\n").unwrap();
        fs::write(dir.join("gadget.md"), "a widget here\n").unwrap();
        fs::write(dir.join("node_modules/widget.js"), "widget").unwrap();
        let root = dir.display().to_string();
        let found = search(&root, "widget").unwrap();
        let names: Vec<&str> = found.names.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(names, vec!["src/widget.rs"]);
        let lines: Vec<(&str, Option<u32>)> = found
            .lines
            .iter()
            .map(|h| (h.path.as_str(), h.line))
            .collect();
        assert!(lines.contains(&("gadget.md", Some(1))), "{lines:?}");
        let gadget = search(&root, "Gadget").unwrap();
        assert_eq!(gadget.names[0].path, "gadget.md");
        assert!(gadget.lines.iter().any(|h| h.path == "src/widget.rs"));
        assert!(search(&root, "   ").unwrap().names.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exact_names_come_first() {
        let dir = temp("rank");
        fs::create_dir_all(dir.join("deep/er")).unwrap();
        fs::write(dir.join("deep/er/main.rs"), "").unwrap();
        fs::write(dir.join("my_main.rs"), "").unwrap();
        fs::write(dir.join("main.rs.bak"), "").unwrap();
        let hits = names(&dir.display().to_string(), "main.rs");
        let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(paths, vec!["deep/er/main.rs", "main.rs.bak", "my_main.rs"]);
        fs::remove_dir_all(&dir).ok();
    }
}
