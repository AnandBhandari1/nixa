use gpui::{App, AppContext, Context, Entity, Global};
use nixa_protocol::paths;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SYSTEM_BIN: &str = "/usr/local/bin";

pub enum Status {
    Installing,
    Installed(PathBuf),
    Unavailable(String),
}

pub struct Installer {
    pub status: Status,
}

struct Shared(Entity<Installer>);

impl Global for Shared {}

pub fn init(cx: &mut App) {
    let installer = cx.new(|_| Installer {
        status: Status::Unavailable(String::new()),
    });
    cx.set_global(Shared(installer.clone()));
    installer.update(cx, Installer::install);
}

pub fn installer(cx: &App) -> Option<Entity<Installer>> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

impl Installer {
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Installing) {
            return;
        }
        self.status = Status::Installing;
        cx.notify();
        let job = cx.background_executor().spawn(async { install() });
        cx.spawn(async move |this, cx| {
            let status = match job.await {
                Ok(path) => Status::Installed(path),
                Err(error) => Status::Unavailable(error),
            };
            this.update(cx, |installer, cx| {
                installer.status = status;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn install() -> Result<PathBuf, String> {
    let source = std::env::current_exe()
        .ok()
        .and_then(|exe| paths::bundled_cli(&exe))
        .ok_or("Install the Nixa app to make its bundled command available.")?;
    let home = paths::home_dir().ok_or("The home folder is unavailable.")?;
    if std::env::var_os(paths::ENV_RELEASE_NONCE).is_some() && home == paths::data_dir() {
        return Err("The command link is disabled for isolated release checks.".into());
    }
    let path = nixa_git::ai::agents::search_path();
    install_in(&source, &home, &path, Path::new(SYSTEM_BIN))
}

fn install_in(source: &Path, home: &Path, path: &str, system: &Path) -> Result<PathBuf, String> {
    let local = home.join(".local/bin");
    let dirs: Vec<_> = std::env::split_paths(path).filter(|dir| dir.is_absolute()).collect();
    let mut errors = Vec::new();
    for dir in [&local, system] {
        let Some(index) = dirs.iter().position(|entry| entry == dir) else {
            continue;
        };
        if let Some(first) = dirs[..index]
            .iter()
            .map(|entry| entry.join(paths::NIXA_BIN_FILE))
            .find(|entry| executable(entry))
        {
            if same_target(&first, source) {
                return Ok(first);
            }
            errors.push(format!("{} takes precedence on PATH", first.display()));
            continue;
        }
        let link = dir.join(paths::NIXA_BIN_FILE);
        let result = if dir == local {
            fs::create_dir_all(dir).and_then(|()| install_link(source, &link))
        } else {
            install_link(source, &link)
        };
        match result {
            Ok(()) => return Ok(link),
            Err(error) => errors.push(format!("{} ({error})", link.display())),
        }
    }
    if errors.is_empty() {
        return Err("Add ~/.local/bin to your terminal PATH, then choose Install again.".into());
    }
    Err(format!("Could not install the command at {}.", errors.join(", ")))
}

fn same_target(link: &Path, source: &Path) -> bool {
    fs::read_link(link).is_ok()
        && fs::canonicalize(link).ok().is_some_and(|path| {
            fs::canonicalize(source).is_ok_and(|source| source == path)
        })
}

fn install_link(source: &Path, link: &Path) -> io::Result<()> {
    if same_target(link, source) {
        return Ok(());
    }
    match symlink(source, link) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        result => return result,
    }
    if same_target(link, source) {
        return Ok(());
    }
    let target = fs::read_link(link).map_err(|_| occupied())?;
    if target.file_name() != Some(OsStr::new(paths::NIXA_BIN_FILE))
        || paths::app_bundle(&target).and_then(Path::file_name) != paths::app_bundle(source).and_then(Path::file_name)
    {
        return Err(occupied());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = NEXT.fetch_add(1, Ordering::Relaxed);
    let pending = link.with_file_name(format!(".nixa-link-{}-{stamp}", std::process::id()));
    symlink(source, &pending)?;
    let result = if fs::read_link(link).is_ok_and(|old| old == target) {
        match fs::rename(&pending, link) {
            Ok(()) => return Ok(()),
            Err(error) => Err(error),
        }
    } else if same_target(link, source) {
        Ok(())
    } else {
        Err(occupied())
    };
    let _ = fs::remove_file(pending);
    result
}

fn occupied() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "an existing command was preserved")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let seq = NEXT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!("nixa-cli-install-{}-{seq}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn home(&self) -> PathBuf {
            self.0.join("home")
        }

        fn local(&self) -> PathBuf {
            self.home().join(".local/bin")
        }

        fn system(&self) -> PathBuf {
            self.0.join("system/bin")
        }

        fn source(&self, copy: &str) -> PathBuf {
            let path = self.0.join(copy).join("Nixa.app/Contents/MacOS/nixa");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        }

        fn install(&self, source: &Path, dirs: &[PathBuf]) -> Result<PathBuf, String> {
            let path = std::env::join_paths(dirs).unwrap();
            install_in(source, &self.home(), path.to_str().unwrap(), &self.system())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn links_the_bundled_command_in_the_home_path_and_refreshes_it() {
        let f = Fixture::new();
        let source = f.source("first");
        assert_eq!(paths::bundled_cli(&source.with_file_name("nixa-app")), Some(source.clone()));
        assert!(paths::bundled_cli(&f.0.join("nixa-app")).is_none());
        let link = f.local().join("nixa");
        assert_eq!(f.install(&source, &[f.local()]).unwrap(), link);
        assert_eq!(fs::read_link(&link).unwrap(), source);
        assert_eq!(f.install(&source, &[f.local()]).unwrap(), link);

        let next = f.source("next");
        fs::remove_file(source).unwrap();
        assert_eq!(f.install(&next, &[f.local()]).unwrap(), link);
        assert_eq!(fs::read_link(&link).unwrap(), next);
    }

    #[test]
    fn preserves_foreign_commands_and_uses_the_other_eligible_path() {
        let f = Fixture::new();
        let source = f.source("app");
        fs::create_dir_all(f.local()).unwrap();
        fs::create_dir_all(f.system()).unwrap();
        let local = f.local().join("nixa");
        fs::write(&local, "an existing file").unwrap();
        let system = f.system().join("nixa");
        assert_eq!(f.install(&source, &[f.local(), f.system()]).unwrap(), system);
        assert_eq!(fs::read_to_string(&local).unwrap(), "an existing file");
        assert_eq!(fs::read_link(&system).unwrap(), source);
        fs::remove_file(&local).unwrap();
        symlink(f.0.join("another-command"), &local).unwrap();
        assert!(f.install(&source, &[f.local()]).unwrap_err().contains("preserved"));
        assert_eq!(fs::read_link(&local).unwrap(), f.0.join("another-command"));
    }

    #[test]
    fn an_earlier_executable_is_reported_and_dirs_off_path_are_untouched() {
        let f = Fixture::new();
        let source = f.source("app");
        let other = f.0.join("other");
        fs::create_dir(&other).unwrap();
        fs::copy(&source, other.join("nixa")).unwrap();
        assert!(f.install(&source, &[other.clone(), f.local()]).unwrap_err().contains("takes precedence"));
        assert!(f.install(&source, &[other]).unwrap_err().contains("terminal PATH"));
        assert!(!f.local().exists());
        assert!(!f.system().exists());
    }

}
