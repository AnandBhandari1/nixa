use crate::brains::BrainStore;
use crate::chat::{Chat, ChatItem, Extra, Who};
use crate::compaction::FoldStore;
use crate::decision::Decision;
use crate::deep::Deep;
use crate::loop_guard::LoopGuard;
use crate::memory::Memory;
use crate::provider::Model;
use crate::spaces::Space;
use crate::trees::TreeStore;
use crate::turns::{Turn, Usage};
use crate::util::{now_ms, read_json, write_json};
use crate::walk::WatchMemory;
use nixa_protocol::{Status, TermInfo};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_NAME: &str = "Nixa";
pub const WATCH_DEFAULT_MINUTES: u64 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub choice: Option<String>,
    pub watch_minutes: Option<u64>,
    pub assistant_name: Option<String>,
    pub deep_auto_start: Option<bool>,
    pub office_soap_model: Option<String>,
    pub office_soap_url: Option<String>,
}

#[derive(Default)]
pub struct BrainState {
    pub turns: Vec<Turn>,
    pub born: Vec<u64>,
    pub queue: Vec<String>,
    pub cancel: Option<CancellationToken>,
    pub busy: bool,
    pub guard: LoopGuard,
    pub models: Vec<Model>,
    pub carry: String,
    pub turn: u64,
    pub last_user: Option<(String, u64)>,
    pub approval_used: Option<(u64, String)>,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub info: TermInfo,
    pub since: u64,
}

#[derive(Default)]
pub struct WatchState {
    pub world: HashMap<String, Seen>,
    pub jobs: HashMap<String, u64>,
    pub memory: WatchMemory,
    pub last_at: u64,
    pub told: HashMap<String, u64>,
}

pub struct App {
    pub dir: PathBuf,
    pub http: reqwest::Client,
    pub chat: Mutex<Chat>,
    pub folds: Mutex<FoldStore>,
    pub brain: Mutex<BrainState>,
    pub brains: BrainStore,
    pub cli: crate::cli_brains::State,
    pub prompts: crate::prompts::State,
    pub voice: crate::voice::State,
    pub voice_map: crate::voice_map::State,
    pub announce: crate::announce::State,
    pub universe: crate::universe::State,
    pub switches: crate::settings_api::State,
    pub modes: crate::modes::State,
    pub custom_agents: crate::custom_agents::State,
    pub schedules: crate::schedules::State,
    pub git_tools: crate::git_tools::State,
    pub project_tools: crate::project_tools::State,
    pub browser: crate::browser::State,
    pub talk: crate::talk::State,
    pub privacy: crate::privacy::State,
    pub soap: crate::soap::State,
    pub phone: crate::phone_door::State,
    pub queue: crate::queue::State,
    pub workflow_writes: Mutex<()>,
    pub review_flow: crate::review_flow::State,
    pub models: crate::models::State,
    pub teams: crate::teams::State,
    pub followup: crate::followup::State,
    pub extras: crate::extras::State,
    pub web: crate::web::State,
    pub knowledge: crate::knowledge::State,
    pub connectors: crate::connectors::State,
    pub desktop: crate::desktop::State,
    pub drones: crate::drones::State,
    pub leave: crate::leave::State,
    pub window_token: String,
    pub gate_token: String,
    pub memory: Memory,
    pub decision: Decision,
    pub deep: Mutex<Deep>,
    pub deep_wake: Arc<tokio::sync::Notify>,
    pub fast_trees: crate::fast_trees::State,
    pub trees: Mutex<TreeStore>,
    pub watch: Mutex<WatchState>,
    pub settings_file: PathBuf,
    settings_write: Mutex<()>,
    pub db_file: PathBuf,
    pub out: broadcast::Sender<Value>,
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl App {
    pub fn open(dir: &Path) -> std::io::Result<Arc<App>> {
        nixa_protocol::paths::ensure_private_dir(dir)?;
        let deep_dir = dir.join("deep");
        nixa_protocol::paths::ensure_private_dir(&deep_dir)?;
        let trees_dir = dir.join("trees");
        nixa_protocol::paths::ensure_private_dir(&trees_dir)?;
        let mut deep = Deep::open(&deep_dir);
        deep.wake_all();
        crate::deep::prepare_files(&deep.rules_file, &deep.memory_file)?;
        let mut trees = TreeStore::open(&trees_dir);
        trees.ensure_first_trees();
        if let Err(e) = crate::spaces::import_old(&dir.join(nixa_store::DB_FILE), &dir.join(nixa_store::spaces::OLD_FILE)) {
            eprintln!("nixa-brain: the old spaces file was not imported: {e:#}");
        }
        let (out, _) = broadcast::channel(512);
        Ok(Arc::new(App {
            dir: dir.to_path_buf(),
            http: crate::provider::client(),
            chat: Mutex::new(Chat::open(Some(&dir.join("chat.json")))),
            folds: Mutex::new(FoldStore::open(Some(&dir.join("compaction.json")))),
            brain: Mutex::new(BrainState::default()),
            brains: BrainStore::open(&dir.join("brains.json")),
            cli: crate::cli_brains::State::open(dir),
            prompts: crate::prompts::State::default(),
            voice: crate::voice::State::open(dir),
            voice_map: crate::voice_map::State::open(dir),
            announce: crate::announce::State::open(dir),
            universe: crate::universe::State::default(),
            switches: crate::settings_api::State::open(dir),
            modes: crate::modes::State::open(dir),
            custom_agents: crate::custom_agents::State::default(),
            schedules: crate::schedules::State::open(dir),
            git_tools: crate::git_tools::State::open(dir),
            project_tools: crate::project_tools::State::default(),
            browser: crate::browser::State::default(),
            talk: crate::talk::State::open(dir),
            privacy: crate::privacy::State::default(),
            soap: crate::soap::State::default(),
            phone: crate::phone_door::State::default(),
            queue: crate::queue::State::default(),
            workflow_writes: Mutex::default(),
            review_flow: crate::review_flow::State::default(),
            models: crate::models::State::default(),
            teams: crate::teams::State::open(dir),
            followup: crate::followup::State::open(dir),
            extras: crate::extras::State::open(dir),
            web: crate::web::State::default(),
            knowledge: crate::knowledge::State::open(dir),
            connectors: crate::connectors::State::open(dir),
            desktop: crate::desktop::State::open(dir),
            drones: crate::drones::State::open(dir),
            leave: crate::leave::State::open(dir),
            window_token: crate::window_key::open(dir),
            gate_token: crate::gate::open(dir),
            memory: Memory::open(&dir.join("memory.md")),
            decision: Decision::open(dir),
            deep: Mutex::new(deep),
            deep_wake: Arc::new(tokio::sync::Notify::new()),
            fast_trees: crate::fast_trees::State::open(dir),
            trees: Mutex::new(trees),
            watch: Mutex::new(WatchState { last_at: now_ms(), ..WatchState::default() }),
            settings_file: dir.join("brain-settings.json"),
            settings_write: Mutex::new(()),
            db_file: dir.join(nixa_store::DB_FILE),
            out,
        }))
    }

    pub fn settings(&self) -> Settings {
        read_json(&self.settings_file).unwrap_or_default()
    }

    pub fn save_settings(&self, change: impl FnOnce(&mut Settings)) -> std::io::Result<Settings> {
        self.try_save_settings(|settings| {
            change(settings);
            Ok(())
        })
    }

    pub(crate) fn try_save_settings(
        &self, change: impl FnOnce(&mut Settings) -> std::io::Result<()>,
    ) -> std::io::Result<Settings> {
        let _held = lock(&self.settings_write);
        let mut settings = self.settings();
        change(&mut settings)?;
        write_json(&self.settings_file, &settings)?;
        Ok(settings)
    }

    pub fn name(&self) -> String {
        self.settings().assistant_name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| DEFAULT_NAME.into())
    }

    pub fn watch_minutes(&self) -> u64 {
        self.settings().watch_minutes.filter(|m| *m <= 240).unwrap_or(WATCH_DEFAULT_MINUTES)
    }

    pub fn spaces(&self) -> Vec<Space> {
        crate::spaces::load(&self.db_file)
    }

    pub fn send_out(&self, value: Value) {
        let _ = self.out.send(value);
    }

    pub fn add_chat(&self, who: Who, text: &str, extra: Extra) -> Option<ChatItem> {
        let item = lock(&self.chat).add(who, text, extra);
        if let Some(item) = &item {
            self.send_out(json!({"type": "chat", "item": item}));
        }
        item
    }

    pub fn info(&self, text: &str) {
        self.add_chat(Who::Info, text, Extra::default());
    }

    pub fn info_in(&self, space: &str, text: &str) {
        self.add_chat(Who::Info, text, Extra { space: Some(space.to_string()), ..Extra::default() });
    }

    pub fn info_for(&self, terminal: &str, text: &str) {
        let space = self.space_of_terminal(terminal).map(|s| s.name);
        self.add_chat(Who::Info, text, Extra { space, ..Extra::default() });
    }

    pub fn terminals(&self) -> Vec<Seen> {
        let mut list: Vec<Seen> = lock(&self.watch).world.values().cloned().collect();
        list.sort_by(|a, b| a.info.id.cmp(&b.info.id));
        list
    }

    pub fn space_of_terminal(&self, terminal: &str) -> Option<Space> {
        let seen = lock(&self.watch).world.get(terminal).cloned()?;
        let spaces = self.spaces();
        if let Some(id) = seen.info.labels.get(crate::deep::SPACE_LABEL)
            && let Some(space) = crate::spaces::find(&spaces, id) {
                return Some(space.clone());
            }
        crate::spaces::for_folder(&spaces, &seen.info.cwd).cloned()
    }
}

pub fn status_word(status: Option<Status>) -> &'static str {
    status.map(|s| s.as_str()).unwrap_or("no status")
}
