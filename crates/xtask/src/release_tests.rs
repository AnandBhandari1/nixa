use super::*;
use std::fs;

fn pair(dir: &Path) -> Record {
    let installed = dir.join("Nixa.app");
    let slot = dir.join("slot/Nixa.app");
    for (path, text) in [(&installed, "old"), (&slot, "new")] {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("marker"), text).unwrap();
    }
    Record {
        old: fingerprint(&installed).unwrap(),
        new: fingerprint(&slot).unwrap(),
        installed,
        slot,
        phase: Phase::Swapping,
        detail: String::new(),
    }
}

#[test]
fn approval_is_exact_and_expires() {
    let token = format!("INSTALL {}", "a".repeat(64));
    assert!(check_approval(&token, "yes\n", Duration::ZERO).is_err());
    assert!(check_approval(&token, &format!("{token} \n"), Duration::ZERO).is_err());
    assert!(check_approval(&token, &format!("{token}\n"), Duration::ZERO).is_ok());
    assert!(check_approval(&token, &token, APPROVAL_TTL + Duration::from_millis(1)).is_err());
    assert!(digest_arg(&"A".repeat(64)).is_err());
}

#[test]
fn journal_explains_interruption_and_excludes_a_second_activation() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let first = lock(dir).unwrap();
    assert!(lock(dir).is_err());
    let mut record = pair(dir);
    record.save(dir, Phase::Swapping, "about to swap").unwrap();
    assert_eq!(
        read(dir).unwrap().unwrap().arrangement(),
        "previous installed; candidate retained"
    );
    exchange(&record.installed, &record.slot).unwrap();
    let recorded = read(dir).unwrap().unwrap();
    assert!(!recorded.phase.complete());
    assert_eq!(
        recorded.arrangement(),
        "candidate installed; previous retained"
    );
    drop(first);
    assert!(lock(dir).is_ok());
}

#[test]
fn verified_rollback_retains_failed_candidate() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let mut record = pair(dir);
    exchange(&record.installed, &record.slot).unwrap();
    let installed = record.installed.clone();
    let err = rollback(
        dir,
        &mut record,
        "window startup failed",
        || Ok(()),
        || {
            assert_eq!(fs::read(installed.join("marker")).unwrap(), b"old");
            Ok(())
        },
    )
    .unwrap_err();
    assert!(err.contains("Previous app and original engine verified"));
    assert_eq!(read(dir).unwrap().unwrap().phase, Phase::RolledBack);
    assert_eq!(fs::read(record.slot.join("marker")).unwrap(), b"new");
}

#[test]
fn stuck_candidate_or_failed_restore_marks_recovery_without_losing_either_bundle() {
    for stop_ok in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        let mut record = pair(dir);
        exchange(&record.installed, &record.slot).unwrap();
        let result = rollback(
            dir,
            &mut record,
            "bad readiness",
            || {
                if stop_ok {
                    Ok(())
                } else {
                    Err("candidate PID did not stop".into())
                }
            },
            || Err("old app startup failed".into()),
        );
        assert!(result.unwrap_err().contains("Recovery required"));
        let saved = read(dir).unwrap().unwrap();
        assert_eq!(saved.phase, Phase::RecoveryRequired);
        assert_eq!(
            saved.arrangement(),
            if stop_ok {
                "previous installed; candidate retained"
            } else {
                "candidate installed; previous retained"
            }
        );
    }
}

#[test]
fn replacing_backup_content_prevents_rollback() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let mut record = pair(dir);
    exchange(&record.installed, &record.slot).unwrap();
    fs::write(record.slot.join("marker"), "wrong backup").unwrap();
    let err = rollback(
        dir,
        &mut record,
        "readiness failed",
        || Ok(()),
        || panic!("must not relaunch"),
    )
    .unwrap_err();
    assert!(err.contains("no rollback swap attempted"));
    assert_eq!(fs::read(record.installed.join("marker")).unwrap(), b"new");
}

#[test]
fn verified_copy_refuses_wrong_digest_without_changing_source() {
    let root = tempfile::tempdir().unwrap();
    let repo = crate::testkit::fake_root(root.path());
    let bins = crate::testkit::fake_release(root.path());
    let src = root.path().join("source.app");
    crate::layout::assemble(&repo, &bins, &src, "0.0.1").unwrap();
    crate::sign::sign(&src, "-").unwrap();
    let digest = candidate(&src).unwrap();
    let copy = root.path().join("copy.app");
    copy_verified(&src, &copy, &digest).unwrap();
    assert_eq!(candidate(&copy).unwrap(), digest);
    assert!(
        copy_verified(&src, &root.path().join("bad.app"), &"a".repeat(64))
            .unwrap_err()
            .contains("differs")
    );
    assert_eq!(candidate(&src).unwrap(), digest);
}

#[test]
#[ignore = "needs current nixa-app, nixa-brain and nixa-engine binaries plus a macOS desktop"]
fn isolated_activation_and_rollback_keep_a_real_engine_running() {
    use std::process::Stdio;

    let bin = PathBuf::from(
        std::env::var_os("NIXA_TEST_RELEASE_BIN_DIR")
            .expect("set NIXA_TEST_RELEASE_BIN_DIR to the current debug binaries"),
    );
    for bad in [false, true] {
        let root = tempfile::Builder::new()
            .prefix("nixa-soak-release-")
            .tempdir_in("/tmp")
            .unwrap();
        let dir = root.path().canonicalize().unwrap();
        let data = dir.join("data");
        fs::create_dir(&data).unwrap();
        let source = dir.join("candidate.app");
        let installed = dir.join("Nixa.app");
        let repo = crate::testkit::fake_root(&dir);
        let stage = crate::testkit::fake_release(&dir);
        for name in ["nixa-brain", "nixa-app", "nixa-engine"] {
            fs::copy(bin.join(name), stage.join(name)).unwrap();
        }
        crate::layout::assemble(&repo, &stage, &installed, "0.0.1").unwrap();
        crate::sign::sign(&installed, "-").unwrap();
        let old = candidate(&installed).unwrap();
        if bad {
            fs::copy("/usr/bin/false", stage.join("nixa-app")).unwrap();
        }
        crate::layout::assemble(&repo, &stage, &source, "0.0.2").unwrap();
        crate::sign::sign(&source, "-").unwrap();
        let digest = candidate(&source).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let runtime = Runtime {
            socket: data.join("engine.sock"),
            data,
            addr,
            wait: Duration::from_secs(12),
        };
        let mut engine = Command::new(bin.join("nixa-app")).arg("engine")
            .arg("--socket")
            .arg(&runtime.socket)
            .env_clear()
            .env("HOME", &runtime.data)
            .env("PATH", "/usr/bin:/bin")
            .env(nixa_protocol::paths::ENV_DATA_DIR, &runtime.data)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut children = Children::default();
        let result = (|| {
            let until = Instant::now() + Duration::from_secs(3);
            let engine_id = loop {
                if let Ok(pid) = runtime.engine_pid() {
                    break pid;
                }
                if Instant::now() > until {
                    return Err("scratch engine did not start".to_string());
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let journal = dir.join("journal");
            if !bad {
                let err = activate(
                    &source,
                    &"0".repeat(64),
                    &installed,
                    &journal,
                    &runtime,
                    |_, _| panic!("unreviewed candidate must not reach approval"),
                    &mut children,
                )
                .unwrap_err();
                assert!(err.contains("changed since inspection"));
                let err = activate(
                    &source,
                    &digest,
                    &installed,
                    &journal,
                    &runtime,
                    |_, _| Err("declined".into()),
                    &mut children,
                )
                .unwrap_err();
                assert_eq!(err, "declined");
                assert!(read(&journal)?.is_none());
                assert_eq!(candidate(&installed)?, old);
                assert_eq!(runtime.engine_pid()?, engine_id);
            }
            let activation = activate(
                &source,
                &digest,
                &installed,
                &journal,
                &runtime,
                |_, _| Ok(()),
                &mut children,
            );
            let mut record = read(&journal)?
                .ok_or_else(|| format!("no recorded transaction: {activation:?}"))?;
            let log =
                fs::read_to_string(record.slot.with_file_name("launch.log")).unwrap_or_default();
            if bad && record.phase != Phase::RolledBack
                || !bad && (record.phase != Phase::Healthy || activation.is_err())
            {
                return Err(format!(
                    "bad={bad}, phase={:?}: {activation:?}\n{log}",
                    record.phase
                ));
            }
            assert_eq!(engine_id, runtime.engine_pid()?);
            assert_eq!(engine_id, engine.id());
            assert_eq!(
                candidate(&installed)?,
                if bad { &old } else { &digest }.as_str()
            );
            assert_eq!(
                candidate(&record.slot)?,
                if bad { &digest } else { &old }.as_str()
            );
            if !bad {
                children.stop()?;
                record.save(
                    &journal,
                    Phase::Checking,
                    "simulate an interruption after exchange",
                )?;
                recover(
                    &installed,
                    &journal,
                    &runtime,
                    &old,
                    |_| Ok(()),
                    &mut children,
                )?;
                assert_eq!(read(&journal)?.unwrap().phase, Phase::RolledBack);
                assert_eq!(candidate(&installed)?, old);
                assert_eq!(candidate(&record.slot)?, digest);
                assert_eq!(runtime.engine_pid()?, engine_id);
            }
            Ok(())
        })();
        let cleanup = children.stop();
        engine.kill().ok();
        engine.wait().ok();
        result.unwrap();
        cleanup.unwrap();
    }
}

#[test]
fn agents_are_sent_to_the_automatic_release() {
    let text = agent_release_text("m3");
    assert!(text.contains("release automatic") && text.contains("nixa deep ask") && text.contains("Terminal m3"), "{text}");
}
