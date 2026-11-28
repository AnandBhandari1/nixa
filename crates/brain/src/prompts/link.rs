use anyhow::{Result, anyhow, bail};
use nixa_protocol::{Key, ReadSource, Reply, Request, TermId, TermInfo, client};
use std::path::PathBuf;
use std::time::Duration;

pub const CALL_SECS: u64 = 15;
pub const READ_LINES: usize = 80;
pub const DRAFT_WAIT_MS: u64 = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub socket: PathBuf,
}

impl Default for Link {
    fn default() -> Link {
        Link { socket: nixa_protocol::paths::socket_path() }
    }
}

pub fn term_id(text: &str) -> Result<TermId> {
    text.trim().parse::<TermId>().map_err(|_| anyhow!("{text} is not a terminal id. Use space-agent-number (like nixa-claude-12) or a legacy id like m1."))
}

impl Link {
    pub fn new(socket: impl Into<PathBuf>) -> Link {
        Link { socket: socket.into() }
    }

    pub async fn call(&self, request: Request, timeout: Duration) -> Result<Reply> {
        let socket = self.socket.clone();
        tokio::task::spawn_blocking(move || {
            let mut client = client::connect(&socket)?;
            client.set_timeout(Some(timeout))?;
            client.call(&request)
        })
        .await
        .map_err(|e| anyhow!("engine call stopped: {e}"))?
    }

    async fn quick(&self, request: Request) -> Result<Reply> {
        self.call(request, Duration::from_secs(CALL_SECS)).await
    }

    pub async fn list(&self) -> Result<Vec<TermInfo>> {
        match self.quick(Request::List { parent: None, label: None }).await? {
            Reply::Terms { terms } => Ok(terms),
            other => bail!("unexpected reply: {other:?}"),
        }
    }

    pub async fn get(&self, terminal: &str) -> Result<TermInfo> {
        match self.quick(Request::Get { id: term_id(terminal)? }).await? {
            Reply::Term { term } => Ok(term),
            other => bail!("unexpected reply: {other:?}"),
        }
    }

    pub async fn read(&self, terminal: &str) -> Result<String> {
        let request = Request::Read { id: term_id(terminal)?, source: ReadSource::Screen, lines: Some(READ_LINES) };
        match self.quick(request).await? {
            Reply::Text { text, .. } => Ok(text),
            other => bail!("unexpected reply: {other:?}"),
        }
    }

    pub async fn keys(&self, terminal: &str, keys: Vec<Key>) -> Result<()> {
        self.quick(Request::Keys { id: term_id(terminal)?, keys }).await.map(|_| ())
    }

    pub async fn send(&self, terminal: &str, text: &str) -> Result<()> {
        let request = Request::Send { id: term_id(terminal)?, text: text.to_string(), submit: true, timeout_ms: Some(DRAFT_WAIT_MS), force: false };
        match self.call(request, Duration::from_secs(CALL_SECS + 10)).await? {
            Reply::Sent { .. } => Ok(()),
            other => bail!("unexpected reply: {other:?}"),
        }
    }
}
