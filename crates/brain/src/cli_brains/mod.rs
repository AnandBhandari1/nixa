pub mod acp;
pub mod direct;
mod claude;
pub mod codex;
mod crush;
pub mod cursor;
pub mod decide_server;
pub mod door;
mod kind;
pub mod opencode;
pub mod proc;
mod process_groups;
mod rpc;
pub mod text_tools;

pub use kind::Kind;

use crate::agent;
use crate::app::{App, lock};
use crate::brains::{EFFORTS_FILE, EffortStore};
use crate::compaction::Context;
use crate::provider::Model;
use crate::reply::AgentReply;
use crate::util::{log, now_ms, read_json, write_json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

pub const SERVER: &str = "nixa_brain";
pub const SETTINGS_FILE: &str = "cli-brains.json";
pub const WORK_DIR: &str = "brain-work";
pub const TOKEN_FILE: &str = nixa_protocol::paths::BRAIN_TOOL_TOKEN_FILE;
pub const TOKEN_HEADER: &str = "x-nixa-token";
pub const CHECK_MS: u64 = 5_000;
pub const RETRY_MS: [u64; 6] = [5_000, 15_000, 30_000, 60_000, 120_000, 300_000];
pub const MODELS_MS: u64 = 90_000;
pub const TURN_IDLE_MS: u64 = 5 * 60 * 1000;
pub const PROMPT_MS: u64 = 10 * 60 * 1000;
pub const REQUEST_MS: u64 = 60_000;
pub const RECAP_ITEMS: usize = 10;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub models: BTreeMap<String, String>,
    pub bins: BTreeMap<String, String>,
    pub text_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpServer {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub bin: PathBuf,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub mcp: McpServer,
    pub instructions: String,
}

#[derive(Debug)]
pub enum Cmd {
    Tell(String),
    Interrupt,
    Reset,
    Stop,
    Model(Option<String>),
    Models(oneshot::Sender<Result<Vec<Model>, String>>),
}

pub struct Handle {
    pub id: u64,
    pub kind: Kind,
    pub tx: mpsc::UnboundedSender<Cmd>,
}

#[derive(Default)]
pub struct Live {
    pub run: Option<Handle>,
    pub next_id: u64,
    pub models: BTreeMap<String, Vec<Model>>,
    pub current_model: Option<String>,
    pub context: Option<Context>,
    pub ready: bool,
    pub down: bool,
    pub fails: u32,
    pub retry_at: u64,
    pub need_recap: bool,
    pub last_request: String,
    pub turn_used_tools: bool,
    pub turn_start: Option<Instant>,
    pub turn_tokens: (u64, u64, u64),
    pub turn_user: Option<bool>,
    pub waiting: Vec<String>,
    pub opencode: Option<(String, String)>,
    pub decide_server: Option<(String, String)>,
    pub decide_used: u64,
}

const STARTED_FILE: &str = "cli-started.json";

pub struct State {
    pub token: String,
    pub token_file: PathBuf,
    pub file: PathBuf,
    pub work_dir: PathBuf,
    pub live: Mutex<Live>,
    groups: process_groups::Groups,
    decide_start: tokio::sync::Mutex<()>,
}

pub fn random_hex(len: usize) -> Option<String> {
    let mut bytes = vec![0u8; len.min(256)];
    let ok = unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } == 0;
    ok.then(|| bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn try_token() -> Option<String> {
    random_hex(24)
}

pub fn new_token() -> String {
    try_token().expect("the system random source failed, so Nixa stops instead of making a weak token")
}

pub fn saved_token(file: &Path, what: &str) -> String {
    let Some(token) = try_token() else {
        log("cli-brain", &format!("no random {what} could be made, so it stays off"));
        return String::new();
    };
    match crate::util::atomic_write(file, &token) {
        Ok(()) => token,
        Err(e) => {
            log("cli-brain", &format!("the {what} file was not written, so it stays off: {e}"));
            String::new()
        }
    }
}

pub fn sweep_work(work_dir: &Path) {
    let Ok(read) = std::fs::read_dir(work_dir) else { return };
    for entry in read.flatten() {
        if entry.file_name().to_string_lossy().starts_with("cursor-") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

impl State {
    pub fn open(dir: &Path) -> State {
        let token_file = dir.join(TOKEN_FILE);
        let token = saved_token(&token_file, "tool token");
        sweep_work(&dir.join(WORK_DIR));
        State { token, token_file, file: dir.join(SETTINGS_FILE), work_dir: dir.join(WORK_DIR), live: Mutex::new(Live::default()), groups: process_groups::Groups::open(dir.join(STARTED_FILE)), decide_start: tokio::sync::Mutex::new(()) }
    }

    pub fn saved(&self) -> Saved {
        read_json(&self.file).unwrap_or_default()
    }

    pub fn save(&self, change: impl FnOnce(&mut Saved)) -> std::io::Result<Saved> {
        let mut saved = self.saved();
        change(&mut saved);
        write_json(&self.file, &saved)?;
        Ok(saved)
    }
}

pub fn efforts(app: &App) -> EffortStore {
    EffortStore::open(&app.dir.join(EFFORTS_FILE))
}

pub fn retry_wait(fails: u32) -> u64 {
    RETRY_MS[(fails.max(1) as usize).min(RETRY_MS.len()) - 1]
}

pub fn chosen(app: &App) -> Option<Kind> {
    app.settings().choice.as_deref().and_then(Kind::parse)
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

pub fn search_dirs(path_var: &str, home: &Path) -> Vec<PathBuf> {
    let home = (!home.as_os_str().is_empty()).then_some(home);
    nixa_protocol::paths::tool_path(path_var, home).split(':').map(PathBuf::from).collect()
}

pub fn find_bin(kind: Kind, saved: &Saved) -> Option<PathBuf> {
    if let Some(path) = saved.bins.get(kind.word()).map(PathBuf::from).filter(|p| is_executable(p)) {
        return Some(path);
    }
    if let Some(path) = std::env::var(kind.bin_env()).ok().map(PathBuf::from).filter(|p| is_executable(p)) {
        return Some(path);
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let path_var = std::env::var("PATH").unwrap_or_default();
    find_path_bin(kind, &path_var, &home)
}

fn find_path_bin(kind: Kind, path: &str, home: &Path) -> Option<PathBuf> {
    let dirs = search_dirs(path, home);
    let plain = dirs.iter().find_map(|dir| {
        let bin = dir.join(kind.bin_name());
        if is_executable(&bin) {
            return Some(bin);
        }
        if kind == Kind::Dim {
            let bin = dir.join(kind.word());
            return is_executable(&bin).then_some(bin);
        }
        None
    });
    if plain.is_some() || kind != Kind::Dim || home.as_os_str().is_empty() {
        return plain;
    }
    std::fs::read_dir(home.join(".t3/tools/dimcode")).ok()?.filter_map(Result::ok)
        .filter_map(|entry| {
            let bin = entry.path().join("npm/bin").join(kind.bin_name());
            let installed = entry.metadata().ok()?.modified().ok()?;
            is_executable(&bin).then_some((installed, bin))
        }).max().map(|(_, bin)| bin)
}

pub fn mcp_command() -> (PathBuf, Vec<String>) {
    if let Some(path) = std::env::var("NIXA_MCP_BIN").ok().filter(|p| !p.is_empty()) {
        return (PathBuf::from(path), vec!["bridge".into()]);
    }
    match std::env::current_exe() {
        Ok(exe) => (exe, vec!["mcp".into(), "bridge".into()]),
        Err(_) => (PathBuf::from("nixa-mcp"), vec!["bridge".into()]),
    }
}

pub fn mcp_server(app: &App, gateway: bool) -> McpServer {
    let mut env = vec![
        ("NIXA_TOOL_URL".to_string(), format!("http://{}", nixa_protocol::paths::brain_addr())),
        ("NIXA_TOOL_TOKEN_FILE".to_string(), app.cli.token_file.display().to_string()),
    ];
    if gateway {
        env.push(("NIXA_MCP_GATEWAY".to_string(), "1".to_string()));
    }
    let (command, args) = mcp_command();
    McpServer { command: command.display().to_string(), args, env }
}

pub fn setup(app: &App, kind: Kind) -> Result<Setup, String> {
    let saved = app.cli.saved();
    let bin = find_bin(kind, &saved).ok_or_else(|| format!("The {} command ({}) is not installed on this Mac.", kind.title(), kind.bin_name()))?;
    nixa_protocol::paths::ensure_private_dir(&app.cli.work_dir).map_err(|e| e.to_string())?;
    Ok(Setup {
        bin,
        cwd: app.cli.work_dir.clone(),
        model: saved.models.get(kind.word()).cloned().filter(|m| !m.is_empty()),
        mcp: mcp_server(app, kind == Kind::Agy),
        instructions: crate::prompt::instructions(app),
    })
}

pub fn recap_text(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = items.iter().map(|l| format!("- {l}")).collect();
    format!("Note: the earlier chat was trimmed to keep replies fast. The last few lines were:\n{}\n\n", lines.join("\n"))
}

pub fn reply_line(kind: &str, chars: usize, took: Option<f32>, tokens: (u64, u64, u64)) -> String {
    let time = took.map(|s| format!(", took {s:.1}s")).unwrap_or_default();
    format!("{kind} reply ({chars} chars{time}, tokens in {} out {} cached {})", tokens.0, tokens.1, tokens.2)
}

pub fn shape_reply(kind: Kind, request: &str, reply: AgentReply) -> AgentReply {
    let reply = agent::recovery::drop_copied("cli-brain", kind.word(), reply);
    if reply.failed && !agent::from_user(request) { AgentReply { speak: false, ..reply } } else { reply }
}

pub fn make_way(running_user: Option<bool>, busy: bool, text: &str) -> bool {
    busy && running_user == Some(false) && agent::from_user(text)
}

pub fn stops_in_place(kind: Kind) -> bool {
    matches!(kind, Kind::Codex | Kind::Agy | Kind::Kimi | Kind::Jcode | Kind::Dim | Kind::Crush | Kind::Omp | Kind::Omo)
}

pub fn deadline(busy: bool, last: Instant) -> Instant {
    if busy { last + Duration::from_millis(TURN_IDLE_MS) } else { Instant::now() + Duration::from_secs(86_400) }
}

#[derive(Clone)]
pub struct Ctx {
    pub app: Arc<App>,
    pub id: u64,
    pub kind: Kind,
}

impl Ctx {
    pub fn mine(&self) -> bool {
        lock(&self.app.cli.live).run.as_ref().is_some_and(|h| h.id == self.id)
    }

    pub fn reply(&self, reply: AgentReply) {
        if !self.mine() {
            return;
        }
        let (request, used_tools) = {
            let mut live = lock(&self.app.cli.live);
            live.turn_user = None;
            (std::mem::take(&mut live.last_request), std::mem::take(&mut live.turn_used_tools))
        };
        if !reply.calls.is_empty() && text_tools::uses(&self.app, self.kind) {
            log("cli-brain", &format!("{} asked for {} text tool call(s)", self.kind.word(), reply.calls.len()));
            text_tools::run(self, reply.calls);
            return;
        }
        let reply = shape_reply(self.kind, &request, reply);
        let order = if reply.failed {
            None
        } else {
            crate::deep::direct_order(&request, used_tools, reply.speak || !reply.text.trim().is_empty())
        };
        text_tools::reset();
        let (took, tokens) = {
            let mut live = lock(&self.app.cli.live);
            (live.turn_start.take().map(|t| t.elapsed().as_secs_f32()), std::mem::take(&mut live.turn_tokens))
        };
        log("cli-brain", &reply_line(self.kind.word(), reply.text.len(), took, tokens));
        agent::deliver(&self.app, reply);
        if let Some(order) = order {
            log("deep", &format!("the quick CLI brain {} ignored a deep brain answer, sending a direct order", self.kind.word()));
            {
                let mut live = lock(&self.app.cli.live);
                live.last_request = order.clone();
                live.turn_used_tools = false;
                live.turn_start = Some(Instant::now());
                live.turn_tokens = (0, 0, 0);
            }
            agent::set_busy(&self.app, true);
            if !send(&self.app, Cmd::Tell(order)) {
                agent::set_busy(&self.app, false);
            }
        }
    }

    pub fn commentary(&self, text: &str) {
        if self.mine() && !text.trim().is_empty() {
            self.app.send_out(json!({"type": "commentary", "text": text.trim()}));
        }
    }

    pub fn busy(&self, on: bool) {
        if !self.mine() {
            return;
        }
        agent::set_busy(&self.app, on);
        if !on {
            let app = self.app.clone();
            tokio::spawn(async move { crate::compact::after_reply(&app).await });
        }
    }

    pub fn turn_started(&self) {
        if !self.mine() {
            return;
        }
        {
            let mut brain = lock(&self.app.brain);
            brain.guard.reset();
            brain.turn += 1;
        }
        lock(&self.app.cli.live).turn_used_tools = false;
        agent::set_busy(&self.app, true);
    }

    pub fn usage(&self, input: u64, output: u64, cached: u64) {
        if !self.mine() {
            return;
        }
        {
            let mut live = lock(&self.app.cli.live);
            live.turn_tokens.0 += input;
            live.turn_tokens.1 += output;
            live.turn_tokens.2 += cached;
        }
        let mut brain = lock(&self.app.brain);
        brain.usage.input += input;
        brain.usage.output += output;
        brain.usage.cached += cached;
    }

    pub fn context(&self, used: u64, window: u64) {
        if self.mine() {
            lock(&self.app.cli.live).context = Some(Context { used, window, estimate: false });
        }
    }

    pub fn models(&self, list: &[Model]) {
        if !self.mine() || list.is_empty() {
            return;
        }
        lock(&self.app.cli.live).models.insert(self.kind.word().to_string(), list.to_vec());
        lock(&self.app.brain).models = list.to_vec();
    }

    pub fn known_models(&self) -> Vec<Model> {
        lock(&self.app.cli.live).models.get(self.kind.word()).cloned().unwrap_or_default()
    }

    pub fn model_now(&self, model: Option<&str>) {
        if self.mine() {
            lock(&self.app.cli.live).current_model = model.map(str::to_string);
        }
    }

    pub fn effort(&self, model: Option<&str>) -> Option<String> {
        let model = model?;
        efforts(&self.app).requested(self.kind.word(), model, None)
    }

    pub fn ready(&self, model: Option<&str>) {
        let back = {
            let mut live = lock(&self.app.cli.live);
            if !live.run.as_ref().is_some_and(|h| h.id == self.id) {
                return;
            }
            let back = live.down || live.fails > 0;
            live.ready = true;
            live.down = false;
            live.fails = 0;
            live.retry_at = 0;
            live.current_model = model.map(str::to_string);
            back
        };
        log("cli-brain", &format!("{} ready, model {}", self.kind.word(), model.unwrap_or("default")));
        if back {
            let name = self.app.name();
            self.app.info(&format!("{name} brain {} is back.", self.kind.title()));
        }
    }

    pub fn trim_note(&self) -> String {
        let name = self.app.name();
        let lines = lock(&self.app.chat).recap(&name, RECAP_ITEMS);
        recap_text(&lines)
    }

    pub fn instructions(&self) -> String {
        crate::prompt::instructions(&self.app)
    }

    pub fn tool_names(&self) -> Vec<String> {
        crate::tools::specs(&self.app).into_iter().map(|s| s.name).collect()
    }
}

fn start(app: &Arc<App>, kind: Kind) -> mpsc::UnboundedSender<Cmd> {
    let (id, tx, rx) = {
        let mut live = lock(&app.cli.live);
        if let Some(handle) = &live.run {
            if handle.kind == kind && !handle.tx.is_closed() {
                return handle.tx.clone();
            }
            let _ = handle.tx.send(Cmd::Stop);
        }
        live.next_id += 1;
        let id = live.next_id;
        let (tx, rx) = mpsc::unbounded_channel();
        live.run = Some(Handle { id, kind, tx: tx.clone() });
        live.ready = false;
        live.context = None;
        live.current_model = None;
        (id, tx, rx)
    };
    let ctx = Ctx { app: app.clone(), id, kind };
    log("cli-brain", &format!("starting {}", kind.word()));
    tokio::spawn(async move {
        let mut rx = rx;
        let out = match kind {
            Kind::Codex => codex::run(&ctx, &mut rx).await,
            Kind::Agy | Kind::Kimi | Kind::Jcode | Kind::Dim => acp::run(&ctx, &mut rx).await,
            Kind::Opencode => opencode::run(&ctx, &mut rx).await,
            Kind::Cursor => cursor::run(&ctx, &mut rx).await,
            Kind::Omp | Kind::Omo => rpc::run(&ctx, &mut rx).await,
            Kind::Claude => claude::run(&ctx, &mut rx).await,
            Kind::Crush => crush::run(&ctx, &mut rx).await,
        };
        rx.close();
        if let Err(error) = &out {
            while let Ok(command) = rx.try_recv() {
                if let Cmd::Models(reply) = command {
                    let _ = reply.send(Err(error.clone()));
                }
            }
        }
        finished(&ctx, out);
    });
    tx
}

fn finished(ctx: &Ctx, out: Result<(), String>) {
    let app = &ctx.app;
    let name = app.name();
    let title = ctx.kind.title();
    if let Err(error) = &out {
        log("cli-brain", &format!("{} ended: {error}", ctx.kind.word()));
        if ctx.mine() && agent::busy(app) {
            let request = lock(&app.cli.live).last_request.clone();
            agent::deliver(app, agent::failed_for(agent::from_user(&request), format!("My {title} brain failed. {error}")));
        }
    }
    let note = {
        let mut live = lock(&app.cli.live);
        if !live.run.as_ref().is_some_and(|h| h.id == ctx.id) {
            return;
        }
        live.run = None;
        let was_ready = live.ready;
        live.ready = false;
        live.context = None;
        match &out {
            Ok(()) => None,
            Err(_) if was_ready => {
                live.down = true;
                live.need_recap = true;
                live.retry_at = now_ms();
                Some(format!("{name} brain {title} stopped. {name} starts it again."))
            }
            Err(error) => {
                live.fails += 1;
                let wait = retry_wait(live.fails);
                live.retry_at = now_ms() + wait;
                (live.fails == 1).then(|| format!("{name} brain {title} did not start: {error} {name} tries again by itself."))
            }
        }
    };
    agent::set_busy(app, false);
    if let Some(note) = note {
        app.info(&note);
    }
}

pub fn running(app: &App) -> Option<Kind> {
    lock(&app.cli.live).run.as_ref().filter(|h| !h.tx.is_closed()).map(|h| h.kind)
}

pub fn send(app: &App, cmd: Cmd) -> bool {
    let live = lock(&app.cli.live);
    match &live.run {
        Some(handle) => handle.tx.send(cmd).is_ok(),
        None => false,
    }
}

pub fn tell(app: &Arc<App>, kind: Kind, text: String) -> bool {
    let mut parts = Vec::new();
    let carry = std::mem::take(&mut lock(&app.brain).carry);
    if !carry.is_empty() {
        parts.push(carry);
    }
    if std::mem::take(&mut lock(&app.cli.live).need_recap) {
        let name = app.name();
        let lines = lock(&app.chat).recap(&name, RECAP_ITEMS);
        if !lines.is_empty() {
            parts.push(crate::chat::restart_note(&name, &lines));
        }
    }
    parts.push(text.clone());
    let busy = agent::busy(app);
    {
        let mut live = lock(&app.cli.live);
        if !busy {
            live.waiting.clear();
        }
        if make_way(live.turn_user, busy, &text) && stops_in_place(kind) && live.run.as_ref().is_some_and(|h| h.kind == kind) {
            log("cli-brain", &format!("{} stopped a background turn so the user message goes first", kind.word()));
            if let Some(handle) = &live.run {
                let _ = handle.tx.send(Cmd::Interrupt);
            }
            live.turn_user = None;
            parts.append(&mut live.waiting);
        } else if busy {
            live.waiting.push(text.clone());
        }
        if !busy || live.turn_user.is_none() && agent::from_user(&text) {
            live.turn_user = Some(agent::from_user(&text));
        }
        live.last_request = text;
        live.turn_used_tools = false;
        live.turn_start = Some(Instant::now());
        live.turn_tokens = (0, 0, 0);
    }
    agent::set_busy(app, true);
    let tx = start(app, kind);
    if tx.send(Cmd::Tell(parts.join("\n\n"))).is_ok() {
        return true;
    }
    agent::set_busy(app, false);
    agent::deliver(app, AgentReply::failed(format!("My {} brain is not running. I try to start it again.", kind.title())));
    false
}

pub fn interrupt(app: &App) -> bool {
    let busy = agent::busy(app);
    if send(app, Cmd::Interrupt) && busy {
        agent::set_busy(app, false);
        return true;
    }
    false
}

pub fn reset(app: &App) {
    send(app, Cmd::Reset);
    lock(&app.cli.live).context = None;
}

pub fn stop(app: &App) {
    let mut live = lock(&app.cli.live);
    if let Some(handle) = live.run.take() {
        let _ = handle.tx.send(Cmd::Stop);
    }
    live.ready = false;
    live.down = false;
    live.fails = 0;
    live.retry_at = 0;
    live.need_recap = false;
    live.context = None;
    live.current_model = None;
}

pub async fn shutdown(app: &App) {
    app.cli.groups.shutdown().await;
    stop(app);
}

pub fn context(app: &App) -> Option<Context> {
    lock(&app.cli.live).context
}

pub fn saved_model(app: &App, kind: Kind) -> Option<String> {
    app.cli.saved().models.get(kind.word()).cloned().filter(|m| !m.is_empty())
}

pub fn model_now(app: &App, kind: Kind) -> Option<String> {
    let live = lock(&app.cli.live);
    let current = live.run.as_ref().filter(|h| h.kind == kind).and(live.current_model.clone());
    drop(live);
    current.or_else(|| saved_model(app, kind))
}

pub async fn models(app: &Arc<App>, kind: Kind) -> Result<Vec<Model>, String> {
    let tx = start(app, kind);
    let (otx, orx) = oneshot::channel();
    tx.send(Cmd::Models(otx)).map_err(|_| format!("The {} brain is not running.", kind.title()))?;
    let list = match tokio::time::timeout(Duration::from_millis(MODELS_MS), orx).await {
        Ok(Ok(result)) => result?,
        Ok(Err(_)) => return Err(format!("The {} brain stopped.", kind.title())),
        Err(_) => return Err(format!("The {} brain did not list its models in time.", kind.title())),
    };
    lock(&app.cli.live).models.insert(kind.word().to_string(), list.clone());
    lock(&app.brain).models = list.clone();
    save_to_cache(app, kind.word(), &list);
    Ok(list)
}

pub fn live_list(list: &[Model]) -> nixa_store::models::AgentModels {
    let models = list.iter().map(|m| m.id.clone()).collect();
    let efforts = list.iter().filter(|m| !m.efforts.is_empty()).map(|m| (m.id.clone(), m.efforts.clone())).collect();
    nixa_store::models::AgentModels { models, efforts }
}

fn save_to_cache(app: &Arc<App>, agent: &str, list: &[Model]) {
    let (db, agent, live) = (app.db_file.clone(), agent.to_string(), live_list(list));
    tokio::task::spawn_blocking(move || {
        if let Ok(conn) = nixa_store::open(&db) {
            let _ = nixa_store::models::lists::save_live(&conn, &agent, &live, now_ms() as i64);
        }
    });
}

pub fn set_model(app: &App, kind: Kind, model: &str) -> Result<(), String> {
    app.cli.save(|s| {
        s.models.insert(kind.word().to_string(), model.to_string());
    })
    .map_err(|e| e.to_string())?;
    if running(app) == Some(kind) {
        lock(&app.cli.live).current_model = Some(model.to_string());
        send(app, Cmd::Model(Some(model.to_string())));
    }
    Ok(())
}

pub fn tick(app: &Arc<App>) {
    let Some(kind) = chosen(app) else {
        if running(app).is_some() {
            stop(app);
        }
        return;
    };
    {
        let live = lock(&app.cli.live);
        if let Some(handle) = &live.run
            && handle.kind == kind && !handle.tx.is_closed() {
                return;
            }
        if now_ms() < live.retry_at {
            return;
        }
    }
    start(app, kind);
}

pub async fn run(app: Arc<App>) {
    loop {
        tick(&app);
        tokio::time::sleep(Duration::from_millis(CHECK_MS)).await;
    }
}

#[cfg(test)]
mod fake_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_waits_grow_to_five_minutes() {
        let waits: Vec<u64> = (0..=8).map(retry_wait).collect();
        assert_eq!(waits, [5_000, 5_000, 15_000, 30_000, 60_000, 120_000, 300_000, 300_000, 300_000]);
    }

    #[test]
    fn kinds_round_trip() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.word()), Some(kind));
        }
        assert_eq!(Kind::parse("api:12345678"), None);
        assert_eq!(Kind::Cursor.bin_name(), "cursor-agent");
        assert_eq!(Kind::Kimi.bin_env(), "NIXA_KIMI_BIN");
        assert_eq!(Kind::Jcode.bin_env(), "NIXA_JCODE_BIN");
        assert_eq!(Kind::Dim.bin_env(), "NIXA_DIM_BIN");
        assert_eq!(Kind::Omp.bin_env(), "NIXA_OMP_BIN");
        assert_eq!(Kind::Omo.bin_env(), "NIXA_OMO_BIN");
        assert_eq!(Kind::parse("omo"), Some(Kind::Omo));
        assert!(rpc::signed_out("Error: Not logged in. Run omo login."));
        assert!(!rpc::signed_out("model list ready"));
        assert_eq!(Kind::Claude.bin_env(), "NIXA_CLAUDE_BIN");
        assert_eq!(Kind::Crush.bin_env(), "NIXA_CRUSH_BIN");
    }

    #[test]
    fn finds_bins_in_known_folders() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(".kimi-code/bin/kimi");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = search_dirs("/nope:/also-nope", dir.path());
        assert!(dirs.iter().any(|d| d.join("kimi") == bin));
        assert!(dirs.contains(&dir.path().join(".bun/bin")));
        assert!(is_executable(&bin));
        assert!(!is_executable(&dir.path().join("missing")));
        let saved = Saved { bins: BTreeMap::from([("kimi".to_string(), bin.display().to_string())]), ..Saved::default() };
        assert_eq!(find_bin(Kind::Kimi, &saved), Some(bin));
    }

    #[test]
    fn dimcode_uses_the_shared_path_and_managed_layout() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join(".t3/tools/dimcode/1.0/npm/bin/dimcode");
        let regular = dir.path().join(".local/bin/dim");
        for path in [&managed, &regular] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(find_path_bin(Kind::Dim, "/missing", dir.path()), Some(regular.clone()));
        std::fs::remove_file(regular).unwrap();
        assert_eq!(find_path_bin(Kind::Dim, "/missing", dir.path()), Some(managed));
    }

    #[test]
    fn tokens_and_recaps() {
        let a = new_token();
        assert_eq!(a.len(), 48);
        assert_ne!(a, new_token());
        assert_eq!(recap_text(&[]), "");
        assert!(recap_text(&["User: hi".into()]).contains("- User: hi"));
    }

    #[test]
    fn token_goes_by_private_file_not_by_value() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let file = std::fs::read_to_string(&app.cli.token_file).unwrap();
        assert_eq!(file, app.cli.token);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&app.cli.token_file).unwrap().permissions().mode() & 0o777, 0o600);
        let server = mcp_server(&app, false);
        assert!(server.env.iter().all(|(_, v)| !v.contains(&app.cli.token)));
        assert!(server.env.iter().any(|(k, v)| k == "NIXA_TOOL_TOKEN_FILE" && v.ends_with(TOKEN_FILE)));
        let args = codex::args(None, &server, &[]);
        assert!(args.iter().all(|a| !a.contains(&app.cli.token)));
    }

    #[test]
    fn saved_settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path());
        state.save(|s| {
            s.models.insert("codex".into(), "m".into());
        })
        .unwrap();
        assert_eq!(state.saved().models["codex"], "m");
    }

    #[test]
    fn direct_order_triggers_when_cli_brain_ignores_deep_answer() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        {
            let mut live = lock(&app.cli.live);
            live.next_id = 1;
            live.run = Some(Handle { id: 1, kind: Kind::Codex, tx });
            live.last_request = format!("{} d12: pick claude", crate::deep::ANSWER_EVENT);
            live.turn_used_tools = false;
        }
        let ctx = Ctx { app: app.clone(), id: 1, kind: Kind::Codex };
        ctx.reply(AgentReply { calls: Vec::new(), speak: false, text: String::new(), failed: false, heard: String::new() });
        match rx.try_recv().unwrap() {
            Cmd::Tell(order) => assert!(order.contains("Direct order") && order.contains("d12"), "{order}"),
            other => panic!("expected direct order Tell, got {other:?}"),
        }
        // When a tool was used, no second direct order is sent.
        {
            let mut live = lock(&app.cli.live);
            live.last_request = format!("{} d13: pick codex", crate::deep::ANSWER_EVENT);
            live.turn_used_tools = true;
        }
        ctx.reply(AgentReply { calls: Vec::new(), speak: false, text: String::new(), failed: false, heard: String::new() });
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn copied_failure_lines_are_dropped_and_background_failures_stay_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        {
            let mut live = lock(&app.cli.live);
            live.next_id = 1;
            live.run = Some(Handle { id: 1, kind: Kind::Opencode, tx });
            live.last_request = "Event: Check-in. Do this now".into();
        }
        let ctx = Ctx { app: app.clone(), id: 1, kind: Kind::Opencode };
        ctx.reply(AgentReply::said(crate::agent::recovery::NOT_RETRIED));
        assert!(lock(&app.chat).list().iter().all(|i| i.who != crate::chat::Who::Nixa));
        let quiet = shape_reply(Kind::Opencode, "Event: Nixa was idle", AgentReply::failed("My OpenCode brain failed. x"));
        assert!(quiet.failed && !quiet.speak);
        let loud = shape_reply(Kind::Opencode, "Typed: \"hi\"", AgentReply::failed("My OpenCode brain failed. x"));
        assert!(loud.failed && loud.speak);
        assert_eq!(shape_reply(Kind::Opencode, "Typed: hi", AgentReply::said("Hello.")).text, "Hello.");
    }

    #[test]
    fn only_a_background_turn_makes_way() {
        assert!(make_way(Some(false), true, "Typed: \"ok\""));
        assert!(!make_way(Some(true), true, "Typed: \"ok\""));
        assert!(!make_way(None, true, "Typed: \"ok\""));
        assert!(!make_way(Some(false), false, "Typed: \"ok\""));
        assert!(!make_way(Some(false), true, "Event: Nixa was idle"));
    }

    #[tokio::test]
    async fn a_user_message_interrupts_a_background_cli_turn() {
        let dir = tempfile::tempdir().unwrap();
        let app = Arc::new(App::open(dir.path()).unwrap());
        app.save_settings(|settings| settings.choice = Some("codex".into())).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        {
            let mut live = lock(&app.cli.live);
            live.next_id = 1;
            live.run = Some(Handle { id: 1, kind: Kind::Codex, tx });
            live.turn_user = Some(false);
        }
        agent::set_busy(&app, true);
        assert!(tell(&app, Kind::Codex, "Event: Done: m3 finished".into()));
        assert!(matches!(rx.try_recv(), Ok(Cmd::Tell(_))));
        assert!(tell(&app, Kind::Codex, "Typed: \"ok\"".into()));
        assert!(matches!(rx.try_recv(), Ok(Cmd::Interrupt)));
        assert!(matches!(rx.try_recv(), Ok(Cmd::Tell(text)) if text.starts_with("Typed") && text.contains("Done: m3")));
        assert_eq!(lock(&app.cli.live).turn_user, Some(true));
        assert!(tell(&app, Kind::Codex, "Heard: \"more\"".into()));
        assert!(matches!(rx.try_recv(), Ok(Cmd::Tell(_))));
        assert!(rx.try_recv().is_err());
        assert!(!stops_in_place(Kind::Claude) && !stops_in_place(Kind::Cursor) && !stops_in_place(Kind::Opencode));
    }
}
