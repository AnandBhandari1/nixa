use super::tree;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

pub type Stamps = BTreeMap<String, Option<SystemTime>>;

pub fn stamp(path: &str) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn stamps(root: &str, folders: &[String], index: Option<&Path>) -> Stamps {
    let mut out: Stamps = folders
        .iter()
        .map(|f| (f.clone(), stamp(&tree::join(root, f))))
        .collect();
    if let Some(index) = index {
        out.insert(
            "\0git".into(),
            fs::metadata(index).and_then(|m| m.modified()).ok(),
        );
    }
    out
}

pub fn changed(old: &Stamps, new: &Stamps) -> bool {
    old != new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_changes_the_folder_stamp() {
        let dir = std::env::temp_dir().join(format!("nixa-files-watch-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("sub")).unwrap();
        let root = dir.display().to_string();
        let folders = vec![String::new(), "sub".to_string()];
        let before = stamps(&root, &folders, None);
        assert!(!changed(&before, &stamps(&root, &folders, None)));
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("sub/new.txt"), "x").unwrap();
        let after = stamps(&root, &folders, None);
        assert!(changed(&before, &after));
        fs::remove_dir_all(&dir).ok();
        assert!(changed(&after, &stamps(&root, &folders, None)));
    }
}
