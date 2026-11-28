pub mod blocker;
pub mod close;
pub mod fix;
pub mod flow;
pub mod hard;
pub mod patches;
pub mod report;
pub mod same_work;
pub mod stall;
pub(crate) mod standalone;
pub(crate) mod targets;

#[cfg(test)]
mod tests;

use crate::app::App;
use close::KeepHolds;
use fix::FixCap;
use hard::HardTurns;
use report::JobFollowUp;
use stall::Stalls;
use targets::Targets;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TICK_MS: u64 = 2000;
pub const EVENT_KIND: &str = "followup";

pub(crate) enum Target {
    Close,
    Fix(Option<Box<standalone::Completion>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowJob {
    pub terminal: String,
    pub at: u64,
    pub sent_at: u64,
    pub revision: u64,
    pub request: String,
    pub sent: Vec<String>,
    pub rounds: u32,
}

pub struct State {
    pub stalls: Mutex<Stalls>,
    pub reports: Mutex<JobFollowUp>,
    pub holds: Mutex<KeepHolds>,
    pub hard: Mutex<HardTurns>,
    pub cap: Mutex<FixCap>,
    pub jobs: Mutex<HashMap<String, FollowJob>>,
    pub next_revision: AtomicU64,
    pub(crate) recovery: Mutex<standalone::Recovery>,
    pub(crate) targets: Mutex<Targets>,
    pub closing: Mutex<HashMap<String, String>>,
    pub fix_asked: Mutex<HashSet<String>>,
    pub replies: Mutex<HashMap<String, String>>,
    pub told_holds: Mutex<HashMap<String, (String, u64)>>,
    pub same_held: Mutex<HashMap<String, u64>>,
    pub returns: Mutex<VecDeque<(String, String)>>,
    pub queued: Mutex<HashMap<String, String>>,
    pub cursor: Mutex<usize>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let targets = Targets::open(&dir.join("deep").join(targets::TARGETS_FILE), crate::util::now_ms());
        let closing = targets.closing();
        let fix_asked = targets.fixing();
        State {
            stalls: Mutex::new(Stalls::default()),
            reports: Mutex::new(JobFollowUp::default()),
            holds: Mutex::new(KeepHolds::open(&dir.join(close::HOLDS_FILE))),
            hard: Mutex::new(HardTurns::default()),
            cap: Mutex::new(FixCap::default()),
            jobs: Mutex::new(HashMap::new()),
            next_revision: AtomicU64::new(1),
            recovery: Mutex::default(),
            targets: Mutex::new(targets),
            closing: Mutex::new(closing),
            fix_asked: Mutex::new(fix_asked),
            replies: Mutex::new(HashMap::new()),
            told_holds: Mutex::new(HashMap::new()),
            same_held: Mutex::new(HashMap::new()),
            returns: Mutex::new(VecDeque::new()),
            queued: Mutex::new(HashMap::new()),
            cursor: Mutex::new(0),
        }
    }
}

pub async fn run(app: Arc<App>) {
    loop {
        tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
        flow::tick(&app).await;
    }
}
