use crate::release_files::{candidate, exchange, fingerprint};
use crate::release_runtime::{self, Children, Runtime};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[path = "release_auto.rs"]
mod automatic;

const DEST: &str = "/Applications/Nixa.app";
const HELP: &str = "cargo run -p xtask -- release <inspect BUNDLE | activate BUNDLE SHA256 | status | recover PREVIOUS_SHA256>";
const JOURNAL: &str = "activation.json";
const APPROVAL_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Swapping,
    Checking,
    RollingBack,
    Healthy,
    RolledBack,
    Aborted,
    RecoveryRequired,
}

impl Phase {
    fn complete(self) -> bool {
        matches!(self, Self::Healthy | Self::RolledBack | Self::Aborted)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    installed: PathBuf,
    slot: PathBuf,
    old: String,
    new: String,
    phase: Phase,
    detail: String,
}

impl Record {
    fn save(&mut self, dir: &Path, phase: Phase, detail: &str) -> Result<(), String> {
        self.phase = phase;
        self.detail = detail.into();
        persist(dir, self)
            .map_err(|e| format!("cannot save release journal at {}: {e}", dir.display()))
    }

    fn arrangement(&self) -> &'static str {
        let installed = fingerprint(&self.installed).ok();
        let slot = fingerprint(&self.slot).ok();
        match (installed.as_deref(), slot.as_deref()) {
            (Some(installed), Some(slot)) if installed == self.old && slot == self.new => {
                "previous installed; candidate retained"
            }
            (Some(installed), Some(slot)) if installed == self.new && slot == self.old => {
                "candidate installed; previous retained"
            }
            _ => "one or both bundles cannot be identified; do not guess or delete either path",
        }
    }
}

fn persist(dir: &Path, record: &Record) -> io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut temp, record)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(dir.join(JOURNAL))?;
    File::open(dir)?.sync_all()
}

fn read(dir: &Path) -> Result<Option<Record>, String> {
    let record = match File::open(dir.join(JOURNAL)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot open release journal: {e}")),
    };
    serde_json::from_reader(record)
        .map(Some)
        .map_err(|e| format!("cannot read release journal: {e}"))
}

fn lock(dir: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join("activation.lock"))
        .map_err(|e| format!("cannot open activation lock: {e}"))?;
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return Err("activation lock is not a regular file".into());
    }
    file.try_lock()
        .map_err(|e| format!("another activation has the lock, or locking failed: {e}"))?;
    Ok(file)
}

fn resolve(bundle: &str) -> Result<PathBuf, String> {
    let path = std::path::absolute(bundle).map_err(|e| e.to_string())?;
    if !path.symlink_metadata().is_ok_and(|meta| meta.is_dir())
        || path.extension().is_none_or(|ext| ext != "app")
    {
        return Err("the staged path must be a real .app directory".into());
    }
    path.canonicalize()
        .map_err(|e| format!("cannot find bundle: {e}"))
}

fn digest_arg(digest: &str) -> Result<(), String> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("use the full lowercase SHA-256 reported by release inspect".into())
    }
}

fn nonce() -> Result<String, String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("could not make a release nonce: {e}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn check_approval(expected: &str, answer: &str, elapsed: Duration) -> Result<(), String> {
    if elapsed > APPROVAL_TTL || answer.trim_end_matches(['\r', '\n']) != expected {
        Err("approval was declined or expired; nothing was installed".into())
    } else {
        Ok(())
    }
}

fn confirm(src: &Path, record: &Record) -> Result<(), String> {
    println!(
        "Candidate: {}\nSHA-256: {}\nInstalled: {}\nPrevious SHA-256: {}\nAutomatic rollback retains both bundles. This does not roll back your data.",
        src.display(),
        record.new,
        record.installed.display(),
        record.old
    );
    prompt(
        "Confirm that this specific release is backward-compatible with your current data",
        &format!("INSTALL {}", record.new),
    )
}

fn prompt(reason: &str, expected: &str) -> Result<(), String> {
    print!("{reason}, then type {expected}: ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut answer = String::new();
    use std::io::Read;
    io::stdin()
        .lock()
        .take(256)
        .read_line(&mut answer)
        .map_err(|e| format!("could not read approval: {e}"))?;
    check_approval(expected, &answer, started.elapsed())
}

fn tell_status(dir: &Path) -> Result<(), String> {
    let Some(record) = read(dir)? else {
        println!("No activation recorded.");
        return Ok(());
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?
    );
    if !record.phase.complete() {
        println!(
            "Recovery required before another activation. Disk state: {}.",
            record.arrangement()
        );
    }
    Ok(())
}

fn copy_verified(src: &Path, slot: &Path, expected: &str) -> Result<(), String> {
    crate::run::text(Command::new("/usr/bin/ditto").arg(src).arg(slot))?;
    if candidate(slot)? != expected {
        return Err("the copied candidate differs from the reviewed fingerprint".into());
    }
    Ok(())
}

fn rollback(
    dir: &Path,
    record: &mut Record,
    cause: &str,
    stop: impl FnOnce() -> Result<(), String>,
    start: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let restore = || -> Result<(), String> {
        stop()?;
        if fingerprint(&record.installed).ok().as_deref() != Some(&record.new)
            || fingerprint(&record.slot).ok().as_deref() != Some(&record.old)
        {
            return Err(
                "the installed or backup bundle changed; no rollback swap attempted".into(),
            );
        }
        record.save(dir, Phase::RollingBack, cause)?;
        exchange(&record.installed, &record.slot)
            .map_err(|e| format!("rollback exchange failed: {e}"))?;
        start()
    };
    match restore() {
        Ok(()) => {
            record.save(dir, Phase::RolledBack, cause)?;
            Err(format!(
                "Candidate failed: {cause}. Previous app and original engine verified. Failed candidate retained at {}",
                record.slot.display()
            ))
        }
        Err(why) => {
            let detail = format!("{cause}; rollback: {why}");
            let _ = record.save(dir, Phase::RecoveryRequired, &detail);
            Err(format!(
                "Recovery required: {detail}. Preserve both {} and {}. Journal: {}",
                record.installed.display(),
                record.slot.display(),
                dir.join(JOURNAL).display()
            ))
        }
    }
}

fn owner_terminal(socket: &Path) -> Result<(), String> {
    let caller = nixa_protocol::caller::who(socket).map_err(|e| e.to_string())?;
    if let nixa_protocol::caller::Caller::Agent(id) = &caller {
        return Err(agent_release_text(id));
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("activation requires a separate interactive terminal outside Nixa".into());
    }
    Ok(())
}

fn agent_release_text(terminal: &str) -> String {
    format!("Terminal {terminal} does not activate or recover a release by hand. The Nixa brain runs release automatic for it with no typed yes: ask Nixa with nixa deep ask to take the release live, then go on with your work. Nothing was changed.")
}

fn recover(
    installed: &Path,
    journal: &Path,
    runtime: &Runtime,
    digest: &str,
    approve: impl FnOnce(&Record) -> Result<(), String>,
    children: &mut Children,
) -> Result<(), String> {
    digest_arg(digest)?;
    let _lock = lock(journal)?;
    let mut record = read(journal)?.ok_or("no activation to recover")?;
    let parent = installed.parent().ok_or("invalid destination")?;
    let slot_dir = record.slot.parent().ok_or("missing recovery folder")?;
    if record.phase.complete()
        || record.old != digest
        || record.installed != installed
        || record.slot.file_name() != installed.file_name()
        || slot_dir.parent() != Some(parent)
        || !slot_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".nixa-release-"))
        || slot_dir.canonicalize().ok().as_deref() != Some(slot_dir)
    {
        return Err("no incomplete activation matches this fingerprint and destination".into());
    }
    runtime.idle(installed, &record.slot)?;
    let engine = runtime.engine_pid()?;
    approve(&record)?;
    let installed_hash = candidate(installed)?;
    let slot_hash = candidate(&record.slot)?;
    let swap = installed_hash == record.new && slot_hash == record.old;
    if !swap && (installed_hash != record.old || slot_hash != record.new) {
        return Err(
            "bundles do not match the journal; preserve both paths for manual recovery".into(),
        );
    }
    runtime.idle(installed, &record.slot)?;
    if runtime.engine_pid()? != engine {
        return Err("terminal engine changed during approval; no files were exchanged".into());
    }
    let log = OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(record.slot.with_file_name("launch.log"))
        .map_err(|e| e.to_string())?;
    record.save(
        journal,
        Phase::RollingBack,
        "owner approved recovery of the previous bundle",
    )?;
    let result = (|| {
        if swap {
            exchange(installed, &record.slot)
                .map_err(|e| format!("recovery exchange failed: {e}"))?;
        }
        runtime.launch(installed, &nonce()?, engine, true, &log, children)
    })();
    if let Err(error) = result {
        let cleanup = children.stop();
        let detail = format!("recovery did not finish: {error}; cleanup: {cleanup:?}");
        record.save(journal, Phase::RecoveryRequired, &detail)?;
        return Err(detail);
    }
    record.save(
        journal,
        Phase::RolledBack,
        "owner recovered the previous bundle and verified the original engine",
    )?;
    println!(
        "Previous release restored. Candidate retained at {}",
        record.slot.display()
    );
    Ok(())
}

fn activate(
    src: &Path,
    digest: &str,
    installed: &Path,
    journal: &Path,
    runtime: &Runtime,
    approve: impl FnOnce(&Path, &Record) -> Result<(), String>,
    children: &mut Children,
) -> Result<(), String> {
    digest_arg(digest)?;
    if installed.canonicalize().ok().as_deref() != Some(installed)
        || src == installed
        || src.starts_with(installed)
    {
        return Err("the destination must be the existing, real Nixa bundle and distinct from the candidate".into());
    }
    runtime.idle(installed, src)?;
    let engine = runtime.engine_pid()?;
    if candidate(src)? != digest {
        return Err("candidate changed since inspection; inspect again".into());
    }
    let old = candidate(installed)?;
    if old == digest {
        return Err("this exact candidate is already installed".into());
    }
    nixa_protocol::paths::ensure_private_dir(journal)
        .map_err(|e| format!("cannot make release journal folder: {e}"))?;
    let _lock = lock(journal)?;
    if let Some(previous) = read(journal)?
        && !previous.phase.complete()
    {
        return Err("the previous activation needs recovery. Run release status first".into());
    }
    let parent = installed.parent().ok_or("invalid install location")?;
    let place = tempfile::Builder::new()
        .prefix(".nixa-release-")
        .tempdir_in(parent)
        .map_err(|e| {
            format!(
                "cannot make an owner-writable backup folder next to {}: {e}",
                installed.display()
            )
        })?;
    if place.path().metadata().map_err(|e| e.to_string())?.dev()
        != installed.metadata().map_err(|e| e.to_string())?.dev()
    {
        return Err("the backup directory and installed bundle are on different volumes".into());
    }
    let slot = place.path().join("Nixa.app");
    let mut record = Record {
        installed: installed.to_owned(),
        slot,
        old,
        new: digest.into(),
        phase: Phase::Prepared,
        detail: String::new(),
    };
    approve(src, &record)?;
    copy_verified(src, &record.slot, digest)?;
    let log = OpenOptions::new()
        .create_new(true)
        .append(true)
        .mode(0o600)
        .open(place.path().join("launch.log"))
        .map_err(|e| e.to_string())?;
    let id = nonce()?;
    let scratch = tempfile::Builder::new()
        .prefix("nixa-preflight-")
        .tempdir_in("/tmp")
        .map_err(|e| e.to_string())?;
    release_runtime::preflight(&record.slot, &id, &scratch.path().join("data"), &log)
        .map_err(|e| format!("scratch preflight failed: {e}"))?;
    if candidate(installed)? != record.old
        || candidate(&record.slot)? != digest
        || runtime.engine_pid()? != engine
    {
        return Err("candidate, installed bundle or terminal engine changed after approval; nothing installed".into());
    }
    runtime.idle(installed, &record.slot)?;
    let retained = place.keep();
    record.save(
        journal,
        Phase::Swapping,
        "verified candidate; atomic exchange about to start",
    )?;
    if let Err(error) = exchange(installed, &record.slot) {
        record.save(journal, Phase::Aborted, &error.to_string())?;
        return Err(format!(
            "exchange failed without replacing the installed app: {error}. Candidate and log retained at {}",
            retained.display()
        ));
    }
    let check = record
        .save(
            journal,
            Phase::Checking,
            "candidate installed; verifying new processes",
        )
        .and_then(|()| runtime.launch(installed, &id, engine, false, &log, children));
    if let Err(cause) = check {
        let copy = record.slot.clone();
        let mut restored = Children::default();
        let result = rollback(
            journal,
            &mut record,
            &cause,
            || {
                children.stop()?;
                runtime.idle(installed, &copy)
            },
            || {
                let result =
                    runtime.launch(installed, &nonce()?, engine, true, &log, &mut restored);
                if result.is_err() {
                    restored.stop()?;
                }
                result
            },
        );
        if record.phase == Phase::RolledBack {
            *children = restored;
        }
        return result;
    }
    record.save(
        journal,
        Phase::Healthy,
        "new window and brain ready on the original terminal engine",
    )?;
    println!(
        "Healthy. Previous signed bundle retained at {}",
        record.slot.display()
    );
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let paths = nixa_protocol::paths::app_data_dir();
    let journal = paths.join("go-live");
    let runtime = || -> Result<Runtime, String> {
        Ok(Runtime {
            socket: paths.join(nixa_protocol::paths::SOCKET_FILE),
            data: paths.clone(),
            addr: nixa_protocol::paths::DEFAULT_BRAIN_ADDR
                .parse()
                .map_err(|_| "invalid default brain address")?,
            wait: Duration::from_secs(25),
        })
    };
    match args {
        [cmd, rest @ ..] if cmd == "automatic" => automatic::run(rest),
        [cmd] if cmd == "status" => tell_status(&journal),
        [cmd, bundle] if cmd == "inspect" => {
            let bundle = resolve(bundle)?;
            println!("{}\nSHA-256: {}", bundle.display(), candidate(&bundle)?);
            Ok(())
        }
        [cmd, bundle, digest] if cmd == "activate" => {
            owner_terminal(&runtime()?.socket)?;
            activate(
                &resolve(bundle)?,
                digest,
                Path::new(DEST),
                &journal,
                &runtime()?,
                confirm,
                &mut Children::default(),
            )
        }
        [cmd, digest] if cmd == "recover" => {
            owner_terminal(&runtime()?.socket)?;
            recover(
                Path::new(DEST),
                &journal,
                &runtime()?,
                digest,
                |record| {
                    println!(
                        "Recovery: {}. {} and {} will both be retained.",
                        record.arrangement(),
                        record.installed.display(),
                        record.slot.display()
                    );
                    prompt(
                        "Confirm recovery of the previous bundle without restoring any data",
                        &format!("RESTORE {digest}"),
                    )
                },
                &mut Children::default(),
            )
        }
        [] => {
            println!("{HELP}");
            Ok(())
        }
        [cmd] if cmd == "help" || cmd == "--help" => {
            println!("{HELP}");
            Ok(())
        }
        _ => Err(HELP.into()),
    }
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
