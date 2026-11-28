use crate::compile::BINS;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub fn temp(tag: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mx-{tag}-{}-{n}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn fake_release(dir: &Path) -> PathBuf {
    let release = dir.join("release");
    fs::create_dir_all(&release).unwrap();
    for (_, bin) in BINS {
        let path = release.join(bin);
        fs::copy("/usr/bin/true", &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    release
}

pub const PAGE_LINE: &str = "const fakePageLineForTheBundleCheck = 'only in the phone page';";

pub fn fake_root(dir: &Path) -> PathBuf {
    let root = dir.join("repo");
    let hooks = root.join("hooks");
    fs::create_dir_all(hooks.join("claude")).unwrap();
    fs::write(hooks.join("README.md"), "read me\n").unwrap();
    fs::write(hooks.join("nixa-status.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(hooks.join("nixa-status.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(hooks.join("claude").join("settings-hooks.json"), "{}\n").unwrap();
    let assets = root.join("crates").join("xtask").join("assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("AppIcon.icns"), b"icns").unwrap();
    fs::write(root.join("THIRD_PARTY_LICENSES.html"), "<html></html>\n").unwrap();
    fs::write(root.join("LICENSE"), "MIT\n").unwrap();
    let page = root.join("phone");
    fs::create_dir_all(&page).unwrap();
    fs::write(page.join("app.js"), format!("let a = 1;\n  {PAGE_LINE}\n")).unwrap();
    fs::write(page.join("index.html"), "<p>short</p>\n").unwrap();
    root
}
