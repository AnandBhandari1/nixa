use crate::compile::{BINS, LINKS};
use crate::opts::{APP_NAME, BUNDLE_ID, app_path};
use crate::plist;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const ICON_ASSET: &str = "crates/xtask/assets/AppIcon.icns";
pub const HOOKS_DIR: &str = "hooks";
pub const HOOK_SCRIPT: &str = "nixa-status.sh";
pub const STAGE_DIR: &str = ".staging";
pub const LICENSE_FILES: [(&str, &str); 2] = [
    ("THIRD_PARTY_LICENSES.html", "THIRD_PARTY_LICENSES.html"),
    ("LICENSE", "LICENSE"),
];

pub fn stage_path(out: &Path) -> PathBuf {
    out.join(STAGE_DIR).join(format!("{APP_NAME}.app"))
}

pub fn contents(app: &Path) -> PathBuf {
    app.join("Contents")
}

pub fn macos_dir(app: &Path) -> PathBuf {
    contents(app).join("MacOS")
}

pub fn resources_dir(app: &Path) -> PathBuf {
    contents(app).join("Resources")
}

pub fn bin_dir(app: &Path) -> PathBuf {
    resources_dir(app).join("bin")
}

pub fn plist_path(app: &Path) -> PathBuf {
    contents(app).join("Info.plist")
}

pub fn icon_path(app: &Path) -> PathBuf {
    resources_dir(app).join(format!("{}.icns", plist::ICON_FILE))
}

fn make_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map(|_| ()).map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || name.ends_with(".md")
}

pub fn copy_tree(from: &Path, to: &Path) -> Result<usize, String> {
    make_dir(to)?;
    let list = fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    let mut count = 0;
    for entry in list.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if skipped(&name) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| format!("cannot read {}: {e}", entry.path().display()))?;
        if kind.is_dir() {
            count += copy_tree(&entry.path(), &to.join(&name))?;
        } else if kind.is_file() {
            copy(&entry.path(), &to.join(&name))?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn missing_licenses(root: &Path) -> Vec<&'static str> {
    LICENSE_FILES.iter().filter(|(from, _)| !root.join(from).is_file()).map(|(from, _)| *from).collect()
}

pub fn license_gate(root: &Path, required: bool) -> Result<Vec<&'static str>, String> {
    let missing = missing_licenses(root);
    if required && !missing.is_empty() {
        return Err(format!(
            "license files are missing in {}: {}. Make them first, or add --allow-missing-licenses for a test bundle",
            root.display(),
            missing.join(", ")
        ));
    }
    Ok(missing)
}

pub fn copy_licenses(root: &Path, resources: &Path) -> Result<Vec<&'static str>, String> {
    let mut copied = Vec::new();
    for (from, to) in LICENSE_FILES {
        let path = root.join(from);
        if path.is_file() {
            copy(&path, &resources.join(to))?;
            copied.push(to);
        }
    }
    Ok(copied)
}

pub fn assemble(root: &Path, release: &Path, stage: &Path, version: &str) -> Result<Vec<&'static str>, String> {
    if stage.exists() {
        fs::remove_dir_all(stage).map_err(|e| format!("cannot clear {}: {e}", stage.display()))?;
    }
    let macos = macos_dir(stage);
    let resources = resources_dir(stage);
    make_dir(&macos)?;
    make_dir(&resources)?;
    for (_, bin) in BINS {
        let from = release.join(bin);
        if !from.is_file() {
            return Err(format!("{} is missing. Build it first, or leave out --no-build", from.display()));
        }
        let to = macos.join(bin);
        copy(&from, &to)?;
        fs::set_permissions(&to, fs::Permissions::from_mode(0o755)).map_err(|e| format!("cannot set {}: {e}", to.display()))?;
    }
    make_dir(&bin_dir(stage))?;
    for (name, word) in LINKS {
        let to = bin_dir(stage).join(name);
        write(&to, &crate::compile::launcher(word))?;
        fs::set_permissions(&to, fs::Permissions::from_mode(0o755)).map_err(|e| format!("cannot set {}: {e}", to.display()))?;
    }
    write(&plist_path(stage), &plist::text(version))?;
    write(&contents(stage).join("PkgInfo"), plist::PKG_INFO)?;
    let icon = root.join(ICON_ASSET);
    if !icon.is_file() {
        return Err(format!("{} is missing. Make it with: cargo run -p xtask -- icon", icon.display()));
    }
    copy(&icon, &icon_path(stage))?;
    let hooks = root.join(HOOKS_DIR);
    if !hooks.join(HOOK_SCRIPT).is_file() {
        return Err(format!("{} is missing", hooks.join(HOOK_SCRIPT).display()));
    }
    copy_tree(&hooks, &resources.join(HOOKS_DIR))?;
    copy_licenses(root, &resources)
}

pub fn is_ours(app: &Path) -> bool {
    fs::read_to_string(plist_path(app)).is_ok_and(|text| text.contains(&format!("<string>{BUNDLE_ID}</string>")))
}

pub fn replace(stage: &Path, out: &Path) -> Result<PathBuf, String> {
    let app = app_path(out);
    if app.exists() {
        if !is_ours(&app) {
            return Err(format!("{} is not a Nixa bundle, so it is not replaced. Move it away first", app.display()));
        }
        fs::remove_dir_all(&app).map_err(|e| format!("cannot remove the old {}: {e}", app.display()))?;
    }
    fs::rename(stage, &app).map_err(|e| format!("cannot move {} to {}: {e}", stage.display(), app.display()))?;
    if let Some(parent) = stage.parent() {
        fs::remove_dir(parent).ok();
    }
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{fake_release, fake_root, temp};

    #[test]
    fn the_bundle_has_programs_plist_icon_and_hooks() {
        let dir = temp("layout");
        let root = fake_root(&dir);
        let release = fake_release(&dir);
        let out = dir.join("out");
        let stage = stage_path(&out);
        for (from, _) in LICENSE_FILES {
            fs::remove_file(root.join(from)).unwrap();
        }
        assert!(assemble(&root, &release, &stage, "0.1.0").unwrap().is_empty());
        assert!(!resources_dir(&stage).join("THIRD_PARTY_LICENSES.html").exists());
        for (_, bin) in BINS {
            let mode = fs::metadata(macos_dir(&stage).join(bin)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{bin}");
        }
        for (name, word) in LINKS {
            assert_eq!(fs::read_to_string(bin_dir(&stage).join(name)).unwrap(), crate::compile::launcher(word), "{name}");
        }
        assert!(is_ours(&stage));
        assert_eq!(fs::read_to_string(contents(&stage).join("PkgInfo")).unwrap(), "APPL????");
        assert_eq!(fs::read(icon_path(&stage)).unwrap(), b"icns");
        let hooks = resources_dir(&stage).join(HOOKS_DIR);
        let mode = fs::metadata(hooks.join(HOOK_SCRIPT)).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
        assert!(hooks.join("claude/settings-hooks.json").is_file());
        assert!(!hooks.join("README.md").exists());
        assert!(!resources_dir(&stage).join("phone").exists());
        let app = replace(&stage, &out).unwrap();
        assert_eq!(app, out.join("Nixa.app"));
        assert!(is_ours(&app) && !stage.exists() && !out.join(STAGE_DIR).exists());
        fs::write(root.join("LICENSE"), "MIT\n").unwrap();
        assert_eq!(assemble(&root, &release, &stage, "0.1.1").unwrap(), ["LICENSE"]);
        fs::write(root.join("THIRD_PARTY_LICENSES.html"), "<html></html>\n").unwrap();
        assert_eq!(assemble(&root, &release, &stage, "0.1.1").unwrap(), ["THIRD_PARTY_LICENSES.html", "LICENSE"]);
        assert_eq!(fs::read_to_string(resources_dir(&stage).join("THIRD_PARTY_LICENSES.html")).unwrap(), "<html></html>\n");
        assert_eq!(fs::read_to_string(resources_dir(&stage).join("LICENSE")).unwrap(), "MIT\n");
        replace(&stage, &out).unwrap();
        assert!(fs::read_to_string(plist_path(&app)).unwrap().contains("0.1.1"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_license_files_stop_a_release_bundle() {
        let dir = temp("licenses");
        let root = fake_root(&dir);
        assert_eq!(license_gate(&root, true).unwrap(), Vec::<&str>::new());
        fs::remove_file(root.join("LICENSE")).unwrap();
        fs::remove_file(root.join("THIRD_PARTY_LICENSES.html")).unwrap();
        assert_eq!(missing_licenses(&root), ["THIRD_PARTY_LICENSES.html", "LICENSE"]);
        let err = license_gate(&root, true).unwrap_err();
        assert!(err.contains("THIRD_PARTY_LICENSES.html, LICENSE") && err.contains("--allow-missing-licenses"), "{err}");
        assert_eq!(license_gate(&root, false).unwrap(), ["THIRD_PARTY_LICENSES.html", "LICENSE"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_folder_that_is_not_ours_is_kept() {
        let dir = temp("keep");
        let out = dir.join("out");
        let app = app_path(&out);
        fs::create_dir_all(contents(&app)).unwrap();
        fs::write(plist_path(&app), "<string>com.example.other</string>").unwrap();
        let stage = stage_path(&out);
        fs::create_dir_all(&stage).unwrap();
        let err = replace(&stage, &out).unwrap_err();
        assert!(err.contains("is not a Nixa bundle"), "{err}");
        assert!(plist_path(&app).is_file() && stage.exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_parts_stop_the_bundle() {
        let dir = temp("missing");
        let root = fake_root(&dir);
        let release = fake_release(&dir);
        fs::remove_file(release.join("nixa-app")).unwrap();
        let err = assemble(&root, &release, &stage_path(&dir), "0.1.0").unwrap_err();
        assert!(err.contains("nixa-app is missing"), "{err}");
        let release = fake_release(&dir);
        fs::remove_file(root.join(ICON_ASSET)).unwrap();
        let err = assemble(&root, &release, &stage_path(&dir), "0.1.0").unwrap_err();
        assert!(err.contains("cargo run -p xtask -- icon"), "{err}");
        fs::remove_dir_all(dir).ok();
    }
}
