pub mod actions;
pub mod agent;
pub mod announce;
pub mod answer_rules;
pub mod anthropic;
pub mod app;
pub mod follow_watch;
pub mod idle_watch;
pub mod app_tools;
pub mod assistant;
pub mod brains;
pub mod browser;
pub mod camera;
pub mod chat;
pub mod cli_brains;
pub mod cli_tool;
pub mod compact;
pub mod compaction;
pub mod connectors;
pub mod context_watch;
pub mod custom_agents;
pub mod decide;
pub mod decision;
pub mod deep;
pub mod deep_service;
pub mod deep_digest;
pub mod desktop;
pub mod drones;
pub mod engine;
pub mod extras;
pub mod git_tools;
pub mod fast_trees;
pub mod flows;
pub mod gate;
pub mod hooks_tool;
pub mod followup;
pub mod joins;
pub mod life;
pub mod keys;
pub mod kid_watch;
pub mod leave;
pub mod knowledge;
pub mod log_file;
pub mod loop_guard;
pub mod manage;
pub mod memory;
pub mod nixa_tools;
pub mod models;
pub mod modes;
pub mod ollama;
pub mod openai;
pub mod prompt;
pub mod persona;
pub mod phone_door;
pub mod privacy;
pub mod project_tools;
pub mod prompts;
pub mod provider;
pub mod queue;
pub mod reply;
pub mod review_flow;
pub mod risk;
pub mod schedules;
pub mod server;
pub mod settings_api;
pub mod soap;
pub mod soul;
pub mod space_tools;
pub mod spaces;
pub mod sse;
pub mod sweep;
pub mod status_rule;
pub mod talk;
pub mod tasks;
pub mod teams;
pub mod term_tools;
pub mod tools;
pub mod tree_api;
pub mod trees;
pub mod turns;
pub mod universe;
pub mod usage_events;
pub mod util;
pub mod voice;
pub mod voice_map;
pub mod walk;
pub mod watch;
pub mod web;
pub mod window_key;
pub mod workflows;
pub mod yes_once;

use anyhow::{Result, bail};

pub const LIVE_PORTS: [&str; 3] = ["7321", "7330", "7331"];
pub const LOCK_FILE: &str = "brain.lock";

pub fn check_addr(addr: &str) -> Result<()> {
    let (host, port) = addr.rsplit_once(':').ok_or_else(|| anyhow::anyhow!("brain address {addr} needs host:port"))?;
    if LIVE_PORTS.contains(&port.trim()) {
        bail!("port {port} is reserved for another local app. Use another port.");
    }
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        bail!("the brain listens on this Mac only. Use 127.0.0.1.");
    }
    Ok(())
}

fn lock_data_dir(dir: &std::path::Path) -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = dir.join(LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            bail!("another nixa-brain already uses {}. Stop it before starting a new one.", dir.display());
        }
        return Err(err.into());
    }
    Ok(file)
}

async fn acquire_runtime(dir: &std::path::Path, addr: &str) -> Result<(std::fs::File, tokio::net::TcpListener)> {
    check_addr(addr)?;
    nixa_protocol::paths::ensure_private_dir(dir)?;
    let lock = lock_data_dir(dir)?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    Ok((lock, listener))
}

pub struct Handle {
    stop: std::sync::Arc<tokio::sync::Notify>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub fn start() -> Result<Handle> {
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let asked = stop.clone();
    let thread = std::thread::Builder::new()
        .name("nixa-brain".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(error) => {
                    util::log("brain", &format!("cannot start the runtime: {error}"));
                    return;
                }
            };
            if let Err(error) = runtime.block_on(run_with(Some(asked))) {
                util::log("brain", &format!("stopped with an error: {error:#}"));
            }
            runtime.shutdown_timeout(std::time::Duration::from_secs(2));
        })?;
    Ok(Handle { stop, thread: Some(thread) })
}

pub fn finished(handle: &Handle) -> bool {
    handle.thread.as_ref().is_none_or(|t| t.is_finished())
}

pub fn stop(handle: &mut Handle, wait: std::time::Duration) -> bool {
    handle.stop.notify_one();
    let until = std::time::Instant::now() + wait;
    while !finished(handle) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if finished(handle) {
        if let Some(thread) = handle.thread.take() {
            let _ = thread.join();
        }
        return true;
    }
    false
}

pub async fn run() -> Result<()> {
    run_with(None).await
}

async fn run_with(asked: Option<std::sync::Arc<tokio::sync::Notify>>) -> Result<()> {
    life::log_panics();
    let addr = nixa_protocol::paths::brain_addr();
    let dir = nixa_protocol::paths::data_dir();
    let (_lock, listener) = acquire_runtime(&dir, &addr).await?;
    let app = app::App::open(&dir)?;
    let made = connectors::builtin::seed(&app.connectors.root);
    if !made.is_empty() {
        util::log("connector", &format!("built-in connectors added: {}", made.join(", ")));
    }
    util::log("gh", &git_tools::gate::ensure(&dir));
    util::log("heavy", &nixa_protocol::heavy::ensure(&dir, nixa_protocol::paths::nixa_bin().as_deref()));
    tokio::spawn(watch::run(app.clone()));
    tokio::spawn(cli_brains::run(app.clone()));
    tokio::spawn(prompts::run(app.clone()));
    tokio::spawn(voice::run(app.clone()));
    tokio::spawn(voice_map::warm(app.clone()));
    tokio::spawn(announce::run(app.clone()));
    tokio::spawn(universe::run(app.clone()));
    tokio::spawn(settings_api::run(app.clone()));
    tokio::spawn(schedules::run(app.clone()));
    tokio::spawn(talk::run(app.clone()));
    tokio::spawn(modes::run(app.clone()));
    tokio::spawn(fast_trees::run(app.clone()));
    tokio::spawn(phone_door::run(app.clone()));
    joins::install();
    tokio::spawn(joins::run(app.clone()));
    tokio::spawn(queue::run(app.clone()));
    tokio::spawn(review_flow::run(app.clone()));
    let model_jobs = tokio::spawn(models::run(app.clone()));
    tokio::spawn(teams::run(app.clone()));
    tokio::spawn(followup::run(app.clone()));
    tokio::spawn(extras::run(app.clone()));
    tokio::spawn(web::run(app.clone()));
    tokio::spawn(knowledge::run(app.clone()));
    tokio::spawn(sweep::run(app.clone()));
    tokio::spawn(manage::go_live::watch(app.clone()));
    util::log("brain", &format!("listening on http://{addr}, data in {}", dir.display()));
    let quit = std::env::var("NIXA_BRAIN_QUIT_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok());
    let signals = life::stop_signal(quit, asked);
    let stop = {
        let app = app.clone();
        async move {
            let why = signals.await;
            util::log("brain", &format!("stopping: {why}"));
            model_jobs.abort();
            let model_lists = tokio::task::spawn_blocking(nixa_store::models::lists::stop_all_lists);
            let (_, result) = tokio::join!(cli_brains::shutdown(&app), model_lists);
            if let Err(error) = result {
                util::log("brain", &format!("model list cleanup failed: {error}"));
            }
        }
    };
    axum::serve(listener, server::router(app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(stop).await?;
    phone_door::stop(&app).await;
    web::stop(&app);
    util::log("brain", "stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_live_port_and_open_hosts() {
        assert!(check_addr("127.0.0.1:7421").is_ok());
        let err = check_addr("127.0.0.1:7321").unwrap_err().to_string();
        assert!(err.contains("reserved for another local app"), "{err}");
        assert!(check_addr("127.0.0.1:7330").is_err());
        assert!(check_addr("localhost:7331").is_err());
        assert!(check_addr("0.0.0.0:7421").is_err());
        assert!(check_addr("nohost").is_err());
    }

    #[tokio::test]
    async fn duplicate_start_fails_before_touching_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let (lock, listener) = acquire_runtime(dir.path(), "127.0.0.1:0").await.unwrap();
        let bound = listener.local_addr().unwrap().to_string();
        let win = dir.path().join(nixa_protocol::paths::WINDOW_TOKEN_FILE);
        let tool = dir.path().join(nixa_protocol::paths::BRAIN_TOOL_TOKEN_FILE);
        std::fs::write(&win, "live-window-token").unwrap();
        std::fs::write(&tool, "live-tool-token").unwrap();

        let err = acquire_runtime(dir.path(), "127.0.0.1:0").await.unwrap_err();
        assert!(err.to_string().contains("already uses"), "{err}");
        assert_eq!(std::fs::read_to_string(&win).unwrap(), "live-window-token");
        assert_eq!(std::fs::read_to_string(&tool).unwrap(), "live-tool-token");

        drop(lock);
        let err = acquire_runtime(dir.path(), &bound).await.unwrap_err();
        assert!(!err.to_string().is_empty());
        assert_eq!(std::fs::read_to_string(&win).unwrap(), "live-window-token");
        assert_eq!(std::fs::read_to_string(&tool).unwrap(), "live-tool-token");

        drop(listener);
        let (_again_lock, _again_listener) = acquire_runtime(dir.path(), &bound).await.unwrap();
    }
}
