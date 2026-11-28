use crate::secrets;
use nixa_protocol::{TermId, paths};
use std::path::PathBuf;

pub const ENV_MODE: &str = "NIXA_MCP_MODE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Full,
    Worker,
}

impl Mode {
    pub fn from_env_value(value: Option<&str>) -> Mode {
        match value.map(str::trim) {
            Some("worker") => Mode::Worker,
            _ => Mode::Full,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Worker => "worker",
        }
    }
}

pub struct State {
    pub mode: Mode,
    pub socket: PathBuf,
    pub data_dir: PathBuf,
    pub own_term: Option<TermId>,
    pub secrets: Vec<String>,
}

impl State {
    pub fn new(mode: Mode, socket: PathBuf, data_dir: PathBuf, own_term: Option<TermId>) -> State {
        State {
            mode,
            socket,
            data_dir,
            own_term,
            secrets: Vec::new(),
        }
    }

    pub fn from_env() -> State {
        let mode = Mode::from_env_value(std::env::var(ENV_MODE).ok().as_deref());
        let mut state = State::new(
            mode,
            paths::socket_path(),
            paths::data_dir(),
            paths::current_term_id(),
        );
        state.secrets = secrets::known_secrets(std::env::vars());
        state
    }
}
