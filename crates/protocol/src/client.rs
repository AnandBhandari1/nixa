use crate::event::Event;
use crate::id::TermId;
use crate::paths;
use crate::reply::{ErrorCode, Reply};
use crate::request::Request;
use crate::wire;
use anyhow::{Context, Result, bail};
use std::fmt;
use std::io::{BufReader, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    pub code: ErrorCode,
    pub message: String,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "engine error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineError {}

pub struct Client {
    path: PathBuf,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    line: String,
    buf: Vec<u8>,
}

pub fn connect(path: &Path) -> Result<Client> {
    let stream = UnixStream::connect(path)
        .with_context(|| format!("cannot reach nixa-engine at {}", path.display()))?;
    let writer = stream.try_clone()?;
    Ok(Client {
        path: path.to_path_buf(),
        reader: BufReader::new(stream),
        writer,
        line: String::new(),
        buf: Vec::with_capacity(256),
    })
}

pub fn connect_default() -> Result<Client> {
    connect(&paths::socket_path())
}

pub fn call_default(req: &Request) -> Result<Reply> {
    connect_default()?.call(req)
}

impl Client {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_idle_alive(&self) -> bool {
        if self.writer.set_nonblocking(true).is_err() {
            return false;
        }
        let mut probe = [0u8; 1];
        let alive = matches!(
            (&self.writer).read(&mut probe),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
        );
        let _ = self.writer.set_nonblocking(false);
        alive
    }

    pub fn set_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.writer.set_read_timeout(timeout)?;
        self.writer.set_write_timeout(timeout)?;
        Ok(())
    }

    pub fn request(&mut self, req: &Request) -> Result<Reply> {
        wire::write_request_buf(&mut self.writer, req, &mut self.buf)?;
        match wire::read_reply_into(&mut self.reader, &mut self.line)? {
            Some(reply) => Ok(reply),
            None => bail!("nixa-engine closed the connection"),
        }
    }

    pub fn call(&mut self, req: &Request) -> Result<Reply> {
        match self.request(req)? {
            Reply::Error { code, message } => Err(EngineError { code, message }.into()),
            reply => Ok(reply),
        }
    }

    pub fn subscribe(
        mut self,
        ids: Vec<TermId>,
        output: bool,
        mut on_event: impl FnMut(Event) -> bool,
    ) -> Result<()> {
        self.set_timeout(None)?;
        match self.call(&Request::Subscribe { ids, output })? {
            Reply::Subscribed => {}
            other => bail!("unexpected reply to subscribe: {other:?}"),
        }
        while let Some(reply) = wire::read_reply_into(&mut self.reader, &mut self.line)? {
            match reply {
                Reply::Event { event } => {
                    if !on_event(event) {
                        return Ok(());
                    }
                }
                Reply::Error { code, message } => return Err(EngineError { code, message }.into()),
                _ => {}
            }
        }
        Ok(())
    }
}
