use super::Hit;
use super::setup::Setup;
use crate::app::lock;
use crate::util::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::{Child, Command};

pub const HOST: &str = "127.0.0.1";
pub const IDLE_MS: u64 = 30 * 60_000;
pub const START_MS: u64 = 15_000;
pub const POLL_MS: u64 = 150;
pub const HEALTH_MS: u64 = 500;
pub const SEARCH_SECS: u64 = 20;
pub const SECRET_FILE: &str = "web-searxng-secret";
pub const COMMAND: &str = ".venv/bin/granian";
pub const NOT_INSTALLED: &str = "Web search is off: local SearXNG is not installed. Install SearXNG in its default folder or set its folder in Settings, or pick Exa with your own key.";
pub const NOT_HEALTHY: &str = "Web search is off: local SearXNG did not start.";
pub const JSON_OFF: &str = "Local SearXNG refused a JSON answer. Add json to the search formats in its settings.yml.";

pub struct Running {
    pub child: Child,
    pub port: u16,
}

#[derive(Default)]
pub struct Keeper {
    pub running: Mutex<Option<Running>>,
    pub starting: tokio::sync::Mutex<()>,
    pub used_at: Mutex<u64>,
}

pub const NO_CLIENT: &str = "The local search link did not start.";

pub fn local_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| NO_CLIENT.to_string())
}

pub fn base(port: u16) -> String {
    format!("http://{HOST}:{port}")
}

pub async fn healthy(client: &reqwest::Client, port: u16) -> bool {
    client
        .get(format!("{}/healthz", base(port)))
        .timeout(Duration::from_millis(HEALTH_MS))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

pub fn command(folder: &Path) -> PathBuf {
    folder.join(COMMAND)
}

pub fn installed(setup: &Setup) -> bool {
    command(&setup.folder).is_file() && setup.settings.is_file()
}

pub fn touch(keeper: &Keeper) {
    *lock(&keeper.used_at) = now_ms();
}

pub fn running(keeper: &Keeper) -> bool {
    let mut slot = lock(&keeper.running);
    match slot.as_mut().map(|r| r.child.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *slot = None;
            false
        }
        None => false,
    }
}

pub fn stop(keeper: &Keeper) -> bool {
    let Some(mut run) = lock(&keeper.running).take() else { return false };
    if let Some(pid) = run.child.id() {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-s", "TERM", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = run.child.start_kill();
    true
}

pub fn needs_restart(before: &Setup, after: &Setup) -> bool {
    !after.on || after.provider != super::setup::Provider::Searxng || before.folder != after.folder || before.settings != after.settings || before.port != after.port
}

pub fn idle_stop(keeper: &Keeper, now: u64) -> bool {
    if !running(keeper) {
        return false;
    }
    let used = *lock(&keeper.used_at);
    now.saturating_sub(used) > IDLE_MS && stop(keeper)
}

fn spawn(setup: &Setup, secret: &str) -> Result<Child, String> {
    Command::new(command(&setup.folder))
        .args(["--interface", "wsgi", "--host", HOST, "--port", &setup.port.to_string(), "searx.webapp:app"])
        .current_dir(&setup.folder)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", super::setup::home())
        .env("LANG", "en_US.UTF-8")
        .env("SEARXNG_SETTINGS_PATH", &setup.settings)
        .env("SEARXNG_SECRET", secret)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| NOT_HEALTHY.to_string())
}

pub async fn ensure(keeper: &Keeper, setup: &Setup, secret: &str) -> Result<(), String> {
    let client = local_client()?;
    let _one = keeper.starting.lock().await;
    if healthy(&client, setup.port).await {
        touch(keeper);
        return Ok(());
    }
    stop(keeper);
    if !installed(setup) {
        return Err(NOT_INSTALLED.into());
    }
    let child = spawn(setup, secret)?;
    *lock(&keeper.running) = Some(Running { child, port: setup.port });
    let until = now_ms() + START_MS;
    while running(keeper) && now_ms() < until {
        if healthy(&client, setup.port).await {
            touch(keeper);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
    stop(keeper);
    Err(NOT_HEALTHY.into())
}

fn random_hex() -> Option<String> {
    crate::cli_brains::random_hex(32)
}

pub fn secret(dir: &Path) -> Result<String, String> {
    let path = dir.join(SECRET_FILE);
    if let Ok(text) = std::fs::read_to_string(&path)
        && text.trim().len() >= 32
    {
        return Ok(text.trim().to_string());
    }
    let made = random_hex().ok_or("No random secret could be made for local SearXNG.")?;
    crate::util::atomic_write(&path, &made).map_err(|_| "The local SearXNG secret was not saved.".to_string())?;
    Ok(made)
}

fn word(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn hits(data: &Value, count: usize) -> Vec<Hit> {
    let Some(list) = data.get("results").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter(|r| super::web_link(&word(r, "url")))
        .take(count)
        .map(|r| {
            let url = word(r, "url");
            let title = Some(word(r, "title")).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| url.clone());
            Hit { title: super::exa::short(&title, 200), url, date: word(r, "publishedDate").chars().take(10).collect(), text: super::exa::short(&word(r, "content"), super::exa::TEXT_CHARS) }
        })
        .collect()
}

pub async fn search(client: &reqwest::Client, port: u16, query: &str, count: usize) -> Result<Vec<Hit>, String> {
    let url = reqwest::Url::parse_with_params(&format!("{}/search", base(port)), &[("q", query), ("format", "json")]).map_err(|_| NOT_HEALTHY.to_string())?;
    let res = client
        .get(url)
        .timeout(Duration::from_secs(SEARCH_SECS))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "Local SearXNG did not answer in time.".to_string() } else { NOT_HEALTHY.to_string() })?;
    match res.status().as_u16() {
        200..=299 => {}
        403 => return Err(JSON_OFF.into()),
        code => return Err(format!("Local SearXNG search failed ({code}).")),
    }
    let data: Value = res.json().await.map_err(|_| "Local SearXNG sent an answer that is not JSON.".to_string())?;
    Ok(hits(&data, count.clamp(1, 10)))
}
