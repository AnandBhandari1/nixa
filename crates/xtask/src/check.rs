use crate::compile::{BINS, LINKS, home};
use crate::layout::{HOOK_SCRIPT, HOOKS_DIR, LICENSE_FILES, bin_dir, icon_path, macos_dir, plist_path, resources_dir};
use crate::plist::NEEDED_KEYS;
use crate::run;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PAGE_DIR: &str = "phone";
pub const PAGE_FILES: [&str; 6] = ["index.html", "app.js", "core.js", "chat-kinds.js", "silence.js", "wake.js"];
pub const PAGE_ROUTES: [&str; 7] = ["/", "/index.html", "/app.js", "/core.js", "/chat-kinds.js", "/silence.js", "/wake.js"];
const PAGE_EXTS: [&str; 2] = ["js", "html"];
const MIN_NEEDLE: usize = 24;
const SYSTEM_LIBS: [&str; 2] = ["/usr/lib/", "/System/Library/"];

pub fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(list) = fs::read_dir(dir) else {
        return found;
    };
    for entry in list.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            found.extend(walk(&entry.path()));
        } else {
            found.push(entry.path());
        }
    }
    found.sort();
    found
}

pub fn page_files(app: &Path) -> Vec<PathBuf> {
    walk(app)
        .into_iter()
        .filter(|path| {
            let named = path.file_name().is_some_and(|n| PAGE_FILES.iter().any(|p| n == *p));
            let in_page_dir = path.strip_prefix(app).is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == PAGE_DIR));
            named || in_page_dir
        })
        .collect()
}

pub fn needle(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.len() >= MIN_NEEDLE)
        .max_by_key(|line| line.len())
        .map(str::to_string)
}

pub fn page_needles(root: &Path) -> Vec<String> {
    walk(&root.join(PAGE_DIR))
        .into_iter()
        .filter(|path| path.parent() == Some(root.join(PAGE_DIR).as_path()))
        .filter(|path| path.extension().is_some_and(|e| PAGE_EXTS.iter().any(|x| e == *x)))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| needle(&text))
        .collect()
}

pub fn build_paths(home: Option<&Path>, root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(home) = home {
        paths.push(format!("{}/", home.display()));
    }
    let root = format!("{}/", root.display());
    if !paths.iter().any(|p| root.starts_with(p.as_str())) {
        paths.push(root);
    }
    paths
}

pub fn scratch_file(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("mx-{tag}-{}-{n}.txt", std::process::id()))
}

pub fn holding(dir: &Path, needles: &[String]) -> Result<Vec<PathBuf>, String> {
    if needles.is_empty() {
        return Ok(Vec::new());
    }
    let list = scratch_file("needles");
    fs::write(&list, needles.join("\n") + "\n").map_err(|e| format!("cannot write {}: {e}", list.display()))?;
    let out = run::output(Command::new("/usr/bin/grep").arg("-rlaF").arg("-f").arg(&list).arg(dir));
    fs::remove_file(&list).ok();
    let out = out?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).lines().map(PathBuf::from).collect()),
        Some(1) => Ok(Vec::new()),
        _ => Err(format!("grep failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

pub fn foreign_libs(otool: &str) -> Vec<String> {
    otool
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.ends_with(':'))
        .map(|line| line.split(" (").next().unwrap_or(line).to_string())
        .filter(|lib| !SYSTEM_LIBS.iter().any(|p| lib.starts_with(p)))
        .collect()
}

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn shown(app: &Path, path: &Path) -> String {
    path.strip_prefix(app).unwrap_or(path).display().to_string()
}

pub fn static_checks(app: &Path, root: &Path, licenses: bool) -> Result<Vec<String>, String> {
    if !app.is_dir() {
        return Err(format!("{} does not exist. Make it with: cargo run -p xtask -- bundle", app.display()));
    }
    let mut problems = Vec::new();
    let macos = macos_dir(app);
    for (_, bin) in BINS {
        if !executable(&macos.join(bin)) {
            problems.push(format!("{bin} is missing in Contents/MacOS"));
        }
    }
    for (link, _) in LINKS {
        if !executable(&bin_dir(app).join(link)) {
            problems.push(format!("{link} is missing in Contents/Resources/bin"));
        }
    }
    let plist = plist_path(app);
    if let Err(why) = run::text(Command::new("/usr/bin/plutil").arg("-lint").arg(&plist)) {
        problems.push(format!("Info.plist is not valid: {why}"));
    }
    let plist_text = fs::read_to_string(&plist).unwrap_or_default();
    for key in NEEDED_KEYS {
        if !plist_text.contains(&format!("<key>{key}</key>")) {
            problems.push(format!("Info.plist has no {key}"));
        }
    }
    if !icon_path(app).is_file() {
        problems.push("the app icon is missing".to_string());
    }
    if !executable(&resources_dir(app).join(HOOKS_DIR).join(HOOK_SCRIPT)) {
        problems.push(format!("Resources/{HOOKS_DIR}/{HOOK_SCRIPT} is missing"));
    }
    if licenses {
        for (_, name) in LICENSE_FILES {
            if !resources_dir(app).join(name).is_file() {
                problems.push(format!("the license file Contents/Resources/{name} is missing"));
            }
        }
    }
    for path in page_files(app) {
        problems.push(format!("the phone page file {} is in the bundle", shown(app, &path)));
    }
    for path in holding(app, &page_needles(root))? {
        problems.push(format!("{} holds text of the phone page", shown(app, &path)));
    }
    for path in holding(app, &build_paths(home().as_deref(), root))? {
        problems.push(format!("{} holds a build folder path", shown(app, &path)));
    }
    for (_, bin) in BINS {
        let path = macos.join(bin);
        if !path.is_file() {
            continue;
        }
        let libs = run::text(Command::new("/usr/bin/otool").arg("-L").arg(&path))?;
        for lib in foreign_libs(&libs) {
            problems.push(format!("{bin} needs {lib}, which is not part of macOS"));
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{assemble, stage_path};
    use crate::testkit::{PAGE_LINE, fake_release, fake_root, temp};

    #[test]
    fn page_files_are_found_by_name_and_folder() {
        let dir = temp("pages");
        let app = dir.join("A.app");
        fs::create_dir_all(app.join("Contents/Resources/phone/img")).unwrap();
        fs::create_dir_all(app.join("Contents/Resources/web")).unwrap();
        fs::write(app.join("Contents/Resources/phone/img/logo.png"), "x").unwrap();
        fs::write(app.join("Contents/Resources/web/wake.js"), "x").unwrap();
        fs::write(app.join("Contents/Resources/web/other.js"), "x").unwrap();
        let found: Vec<String> = page_files(&app).iter().map(|p| shown(&app, p)).collect();
        assert_eq!(found, ["Contents/Resources/phone/img/logo.png", "Contents/Resources/web/wake.js"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_longest_page_line_is_the_needle() {
        assert_eq!(needle("short\n   a line that is long enough to count   \n  x\n"), Some("a line that is long enough to count".into()));
        assert_eq!(needle("tiny\nlines\n"), None);
        let dir = temp("needles");
        let root = fake_root(&dir);
        fs::create_dir_all(root.join("phone/tests")).unwrap();
        fs::write(root.join("phone/tests/deep.js"), "a test line that is long enough to be a needle\n").unwrap();
        assert_eq!(page_needles(&root), [PAGE_LINE]);
        assert!(page_needles(&dir.join("nothing")).is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn build_paths_cover_home_once() {
        let home = Path::new("/Users/someone");
        assert_eq!(build_paths(Some(home), Path::new("/Users/someone/src/m")), ["/Users/someone/"]);
        assert_eq!(build_paths(Some(home), Path::new("/opt/m")), ["/Users/someone/", "/opt/m/"]);
        assert_eq!(build_paths(None, Path::new("/opt/m")), ["/opt/m/"]);
    }

    #[test]
    fn only_system_libraries_are_allowed() {
        let text = "/x/nixa-app:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n\t/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit (compatibility version 45.0.0)\n\t@rpath/libonnxruntime.dylib (compatibility version 0.0.0)\n\t/opt/homebrew/lib/libz.dylib (compatibility version 1.0.0)\n";
        assert_eq!(foreign_libs(text), ["@rpath/libonnxruntime.dylib", "/opt/homebrew/lib/libz.dylib"]);
    }

    #[test]
    fn a_clean_bundle_passes_and_planted_problems_are_found() {
        let dir = temp("check");
        let root = fake_root(&dir);
        let release = fake_release(&dir);
        let app = stage_path(&dir.join("out"));
        assemble(&root, &release, &app, "0.1.0").unwrap();
        assert_eq!(static_checks(&app, &root, true).unwrap(), Vec::<String>::new());
        fs::remove_file(resources_dir(&app).join("LICENSE")).unwrap();
        fs::remove_file(resources_dir(&app).join("THIRD_PARTY_LICENSES.html")).unwrap();
        assert_eq!(
            static_checks(&app, &root, true).unwrap(),
            ["the license file Contents/Resources/THIRD_PARTY_LICENSES.html is missing", "the license file Contents/Resources/LICENSE is missing"]
        );
        assert_eq!(static_checks(&app, &root, false).unwrap(), Vec::<String>::new());
        let brain = macos_dir(&app).join("nixa-app");
        let mut bytes = fs::read(&brain).unwrap();
        bytes.extend_from_slice(PAGE_LINE.as_bytes());
        bytes.extend_from_slice(format!("{}/x", root.display()).as_bytes());
        fs::write(&brain, bytes).unwrap();
        fs::create_dir_all(resources_dir(&app).join("phone")).unwrap();
        fs::write(resources_dir(&app).join("phone/core.js"), "x").unwrap();
        fs::remove_file(bin_dir(&app).join("nixa-mcp")).unwrap();
        let problems = static_checks(&app, &root, false).unwrap();
        assert_eq!(
            problems,
            [
                "nixa-mcp is missing in Contents/Resources/bin",
                "the phone page file Contents/Resources/phone/core.js is in the bundle",
                "Contents/MacOS/nixa-app holds text of the phone page",
                "Contents/MacOS/nixa-app holds a build folder path",
            ]
        );
        assert!(static_checks(&dir.join("none.app"), &root, true).unwrap_err().contains("does not exist"));
        fs::remove_dir_all(dir).ok();
    }
}
