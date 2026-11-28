use anyhow::{Context, Result, anyhow, bail};
use nixa_protocol::{CreateSpec, EngineError, Event, InterruptReport, Key, ReadSource, Reply, Request, TermId, TermInfo, WaitFor, WaitResult, client, paths, wire};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::runtime::{Handle, Id as RuntimeId};
use tokio::sync::mpsc;

pub const CALL_SECS: u64 = 15;
const MAX_IDLE: usize = 8;

struct Conn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    line: String,
    buf: Vec<u8>,
}

struct Pooled {
    path: PathBuf,
    runtime: RuntimeId,
    conn: Conn,
}

static POOL: LazyLock<Mutex<Vec<Pooled>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn is_idle_alive(conn: &Conn) -> bool {
    let mut probe = [0u8; 1];
    matches!(conn.reader.get_ref().try_read(&mut probe), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
}

fn take_conn(path: &Path, runtime: RuntimeId) -> Option<Conn> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    while let Some(pos) = pool.iter().rposition(|p| p.runtime == runtime && p.path == path) {
        let conn = pool.swap_remove(pos).conn;
        if is_idle_alive(&conn) {
            return Some(conn);
        }
    }
    None
}

fn put_conn(path: PathBuf, runtime: RuntimeId, conn: Conn) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.retain(|p| p.runtime == runtime);
    if pool.len() < MAX_IDLE {
        pool.push(Pooled { path, runtime, conn });
    }
}

async fn connect_async(path: &Path) -> Result<Conn> {
    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("cannot reach nixa-engine at {}", path.display()))?;
    let (read_half, writer) = stream.into_split();
    Ok(Conn {
        reader: BufReader::new(read_half),
        writer,
        line: String::new(),
        buf: Vec::with_capacity(256),
    })
}

async fn write_req(conn: &mut Conn, req: &Request) -> std::io::Result<()> {
    conn.buf.clear();
    serde_json::to_writer(&mut conn.buf, req)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    conn.buf.push(b'\n');
    conn.writer.write_all(&conn.buf).await?;
    conn.writer.flush().await
}

async fn read_reply(conn: &mut Conn) -> std::io::Result<Option<Reply>> {
    let cap = wire::MAX_LINE;
    let limit = u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1);
    loop {
        conn.line.clear();
        let n = (&mut conn.reader).take(limit).read_line(&mut conn.line).await?;
        if n == 0 {
            return Ok(None);
        }
        if conn.line.len() > cap {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "line too long"));
        }
        if !conn.line.trim().is_empty() {
            return wire::parse_reply(&conn.line).map(Some);
        }
    }
}

fn check_reply(reply: Option<Reply>) -> Result<Reply> {
    match reply {
        Some(Reply::Error { code, message }) => Err(EngineError { code, message }.into()),
        Some(reply) => Ok(reply),
        None => bail!("nixa-engine closed the connection"),
    }
}

pub async fn call(request: Request, timeout: Duration) -> Result<Reply> {
    let path = paths::socket_path();
    let runtime = Handle::current().id();
    let work = async {
        if let Some(mut conn) = take_conn(&path, runtime)
            && write_req(&mut conn, &request).await.is_ok()
        {
            let reply = read_reply(&mut conn).await?;
            if reply.is_some() {
                put_conn(path, runtime, conn);
            }
            return check_reply(reply);
        }
        let mut conn = connect_async(&path).await?;
        write_req(&mut conn, &request).await?;
        let reply = read_reply(&mut conn).await?;
        if reply.is_some() {
            put_conn(path, runtime, conn);
        }
        check_reply(reply)
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| anyhow!("nixa-engine did not answer in {} ms", timeout.as_millis()))?
}

pub async fn quick(request: Request) -> Result<Reply> {
    call(request, Duration::from_secs(CALL_SECS)).await
}

pub fn term_id(text: &str) -> Result<TermId> {
    text.trim().parse::<TermId>().map_err(|_| anyhow!("{text} is not a terminal id. Use space-agent-number (like nixa-claude-12) or a legacy id like m1."))
}

pub async fn list() -> Result<Vec<TermInfo>> {
    match quick(Request::List { parent: None, label: None }).await? {
        Reply::Terms { terms } => Ok(terms),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn get(id: &TermId) -> Result<TermInfo> {
    match quick(Request::Get { id: id.clone() }).await? {
        Reply::Term { term } => Ok(term),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn read(id: &TermId, lines: Option<usize>, history: bool) -> Result<String> {
    let source = if history { ReadSource::History } else { ReadSource::Screen };
    match quick(Request::Read { id: id.clone(), source, lines }).await? {
        Reply::Text { text, .. } => Ok(text),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn send(id: &TermId, text: &str, submit: bool, force: bool) -> Result<u64> {
    send_with(id, text, submit, force, None).await
}

pub(crate) async fn send_now(id: &TermId, text: &str) -> Result<u64> {
    send_with(id, text, true, false, Some(0)).await
}

pub struct SendReport {
    pub waited_ms: u64,
    pub started: Option<bool>,
    pub tail: String,
}

pub async fn send_report(id: &TermId, text: &str, submit: bool, force: bool) -> Result<SendReport> {
    let request = Request::Send { id: id.clone(), text: text.to_string(), submit, timeout_ms: None, force };
    let wait = Duration::from_millis(nixa_protocol::DEFAULT_SEND_TIMEOUT_MS + 10_000);
    match call(request, wait).await? {
        Reply::Sent { waited_ms, started, tail, .. } => {
            crate::universe::sent(&id.to_string(), text);
            Ok(SendReport { waited_ms, started, tail })
        }
        other => bail!("unexpected reply: {other:?}"),
    }
}

async fn send_with(id: &TermId, text: &str, submit: bool, force: bool, timeout_ms: Option<u64>) -> Result<u64> {
    let request = Request::Send { id: id.clone(), text: text.to_string(), submit, timeout_ms, force };
    let wait = Duration::from_millis(timeout_ms.unwrap_or(nixa_protocol::DEFAULT_SEND_TIMEOUT_MS) + 10_000);
    match call(request, wait).await? {
        Reply::Sent { waited_ms, .. } => {
            crate::universe::sent(&id.to_string(), text);
            Ok(waited_ms)
        }
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn keys(id: &TermId, keys: Vec<Key>) -> Result<()> {
    quick(Request::Keys { id: id.clone(), keys }).await.map(|_| ())
}

pub async fn interrupt(id: &TermId) -> Result<InterruptReport> {
    match quick(Request::Interrupt { id: id.clone() }).await? {
        Reply::Interrupted { report } => Ok(report),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn create(spec: CreateSpec) -> Result<TermInfo> {
    match quick(Request::Create { spec }).await? {
        Reply::Term { term } => Ok(term),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn remove(id: &TermId) -> Result<()> {
    quick(Request::Remove { id: id.clone() }).await.map(|_| ())
}

pub async fn label(id: &TermId, key: &str, value: Option<&str>) -> Result<()> {
    quick(Request::LabelSet { id: id.clone(), key: key.to_string(), value: value.map(str::to_string) }).await.map(|_| ())
}

pub async fn wait(id: &TermId, until: WaitFor, timeout_ms: u64) -> Result<WaitResult> {
    let request = Request::Wait { id: id.clone(), until, timeout_ms: Some(timeout_ms) };
    match call(request, Duration::from_millis(timeout_ms + 10_000)).await? {
        Reply::Waited { result } => Ok(result),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub async fn ping() -> Result<u32> {
    match quick(Request::Ping).await? {
        Reply::Pong { version, .. } => Ok(version),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub fn watch_events(tx: mpsc::UnboundedSender<Event>) {
    std::thread::spawn(move || {
        let mut said = false;
        loop {
            if tx.is_closed() {
                return;
            }
            let result = client::connect(&paths::socket_path()).and_then(|c| {
                said = false;
                c.subscribe(Vec::new(), false, |event| tx.send(event).is_ok())
            });
            if let Err(error) = result
                && !said {
                    crate::util::log("engine", &format!("event stream down, trying again: {error}"));
                    said = true;
                }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}
