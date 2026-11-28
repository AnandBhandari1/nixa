use nixa_protocol::TermId;
use crate::counter;
use nixa_protocol::id::slug;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub fn space_for(label: Option<&str>, inherited: Option<&str>, cwd: &Path, home: &Path) -> String {
    label
        .into_iter()
        .chain(inherited)
        .map(slug)
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cwd == home {
                return "local".into();
            }
            cwd.file_name()
                .and_then(|name| name.to_str())
                .map(slug)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "local".into())
        })
}

pub fn agent_for(program: &str) -> String {
    match program {
        "agy" => "antigravity".into(),
        "cursor-agent" => "cursor".into(),
        _ => {
            let name = slug(program);
            if name.is_empty() {
                "shell".into()
            } else {
                name
            }
        }
    }
}

pub const NUMBER_MARK: &str = "number-";

fn claim(dir: &Path, name: &str) -> io::Result<bool> {
    match OpenOptions::new().write(true).create_new(true).mode(0o600).open(dir.join(name)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

fn highest(dir: &Path) -> io::Result<u64> {
    let mut top = 0;
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        if let Some(n) = name.to_str().and_then(|n| n.strip_prefix(NUMBER_MARK)).and_then(|n| n.parse::<u64>().ok()) {
            top = top.max(n);
        }
    }
    Ok(top)
}

pub fn reserve(dir: &Path, counter_file: &Path, space: &str, agent: &str) -> io::Result<TermId> {
    fs::create_dir_all(dir)?;
    let mut number = counter::load(counter_file).max(highest(dir)? + 1);
    for _ in 0..32 {
        let n = number;
        number += 1;
        if !claim(dir, &format!("{NUMBER_MARK}{n}"))? {
            continue;
        }
        let id: TermId = format!("{space}-{agent}-{n}")
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if !claim(dir, id.as_str())? {
            continue;
        }
        if let Err(error) = counter::save(counter_file, number) {
            eprintln!("nixa-engine: cannot save the terminal counter: {error}");
        }
        return Ok(id);
    }
    Err(io::Error::other("could not allocate a unique terminal id"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_use_the_next_terminal_number_once_even_after_a_restart_or_a_lost_counter() {
        let dir = std::env::temp_dir().join(format!("nixa-terminal-numbers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ids = dir.join("terminal-ids");
        let counter_file = dir.join(counter::COUNTER_FILE);
        assert_eq!(reserve(&ids, &counter_file, "nixa", "claude").unwrap().as_str(), "nixa-claude-1");
        assert_eq!(reserve(&ids, &counter_file, "ace", "codex").unwrap().as_str(), "ace-codex-2");
        assert_eq!(counter::load(&counter_file), 3);
        fs::remove_file(&counter_file).unwrap();
        assert_eq!(reserve(&ids, &counter_file, "nixa", "claude").unwrap().as_str(), "nixa-claude-3");
        counter::save(&counter_file, 2).unwrap();
        assert_eq!(reserve(&ids, &counter_file, "nixa", "sh").unwrap().as_str(), "nixa-sh-4");
        fs::write(ids.join("nixa-claude-5"), "").unwrap();
        assert_eq!(reserve(&ids, &counter_file, "nixa", "claude").unwrap().as_str(), "nixa-claude-6");
        let threads: Vec<_> = (0..8).map(|_| {
            let (ids, counter_file) = (ids.clone(), counter_file.clone());
            std::thread::spawn(move || reserve(&ids, &counter_file, "nixa", "claude").unwrap())
        }).collect();
        let mut made: Vec<String> = threads.into_iter().map(|t| t.join().unwrap().to_string()).collect();
        made.sort();
        made.dedup();
        assert_eq!(made.len(), 8, "{made:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spaces_use_label_then_parent_then_directory_without_exposing_home_basename() {
        let home = Path::new("/test-home");
        let project = Path::new("/workspace/Fresh Project");
        assert_eq!(
            space_for(Some("My Space"), Some("parent"), project, home),
            "my-space"
        );
        assert_eq!(
            space_for(Some("!"), Some("parent"), project, home),
            "parent"
        );
        assert_eq!(space_for(None, None, project, home), "fresh-project");
        assert_eq!(space_for(None, None, home, home), "local");
    }
}
