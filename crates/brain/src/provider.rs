use crate::sse::SseBuf;
use crate::turns::{StepResult, ToolSpec, Turn};
use crate::{anthropic, openai};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_MAX_TOKENS: u32 = 4096;
pub const REQUEST_SECS: u64 = 120;
pub const CONNECT_SECS: u64 = 30;
pub const MODELS_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Openai,
    Anthropic,
    Ollama,
}

impl Kind {
    pub fn parse(word: &str) -> Option<Kind> {
        match word {
            "openai" => Some(Kind::Openai),
            "anthropic" => Some(Kind::Anthropic),
            "ollama" => Some(Kind::Ollama),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Kind::Openai => "openai",
            Kind::Anthropic => "anthropic",
            Kind::Ollama => "ollama",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Openai => "OpenAI",
            Kind::Anthropic => "Anthropic",
            Kind::Ollama => "Ollama",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Kind::Openai => "https://api.openai.com/v1",
            Kind::Anthropic => "https://api.anthropic.com",
            Kind::Ollama => "http://127.0.0.1:11434/v1",
        }
    }
}

#[derive(Clone)]
pub struct Endpoint {
    pub kind: Kind,
    pub base_url: String,
    pub api_key: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint").field("kind", &self.kind).field("base_url", &self.base_url).field("api_key", &"<hidden>").finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

pub fn limit_of(models: &[Model], id: &str) -> Option<u32> {
    models.iter().find(|m| m.id == id).and_then(|m| m.max_tokens)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub status: Option<u16>,
    pub message: String,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&plain_error(self.status, &self.message))
    }
}

impl std::error::Error for ProviderError {}

pub struct StepInput<'a> {
    pub system: &'a str,
    pub turns: &'a [Turn],
    pub tools: &'a [ToolSpec],
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub max_tokens: Option<u32>,
}

pub fn same_host_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let first = attempt.previous().first().map(|u| (u.scheme().to_string(), u.host_str().map(str::to_string), u.port_or_known_default()));
        let next = (attempt.url().scheme().to_string(), attempt.url().host_str().map(str::to_string), attempt.url().port_or_known_default());
        if attempt.previous().len() > 10 || first.as_ref() != Some(&next) { attempt.stop() } else { attempt.follow() }
    })
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_SECS))
        .read_timeout(Duration::from_secs(REQUEST_SECS))
        .redirect(same_host_redirects())
        .build()
        .unwrap_or_default()
}

pub fn plain_error(status: Option<u16>, text: &str) -> String {
    match status {
        Some(401) | Some(403) => return format!("The key was refused ({}).", status.unwrap_or(0)),
        Some(404) => return "The URL or the model was not found (404).".into(),
        Some(429) => return "The server says too many requests, or there is no credit left (429).".into(),
        _ => {}
    }
    let low = text.to_lowercase();
    if low.contains("does not support tools") || low.contains("tools are not supported") || low.contains("tool use is not supported") {
        return "This model cannot use tools. Pick another model.".into();
    }
    if low.contains("connection refused") || low.contains("dns error") || low.contains("error sending request") || low.contains("connect error") {
        return "Could not reach the URL.".into();
    }
    if low.contains("timed out") || low.contains("timeout") || low.contains("aborted") {
        return "The server did not answer in time.".into();
    }
    let clean = crate::util::flat(text);
    if clean.chars().count() > 160 { format!("{}…", clean.chars().take(160).collect::<String>()) } else { clean }
}

fn join_url(base: &str, tail: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), tail.trim_start_matches('/'))
}

fn anthropic_base(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1") { trimmed.to_string() } else { format!("{trimmed}/v1") }
}

pub(crate) fn with_auth(request: reqwest::RequestBuilder, endpoint: &Endpoint) -> reqwest::RequestBuilder {
    match endpoint.kind {
        Kind::Ollama if endpoint.api_key.is_empty() => request,
        Kind::Openai | Kind::Ollama => request.bearer_auth(if endpoint.api_key.is_empty() { "none" } else { &endpoint.api_key }),
        Kind::Anthropic => {
            let request = request.header("anthropic-version", anthropic::VERSION);
            if endpoint.api_key.is_empty() {
                request
            } else if anthropic::is_official(&endpoint.base_url) {
                request.header("x-api-key", &endpoint.api_key)
            } else {
                request.header("x-api-key", &endpoint.api_key).bearer_auth(&endpoint.api_key)
            }
        }
    }
}

fn net_error(error: reqwest::Error) -> ProviderError {
    ProviderError { status: error.status().map(|s| s.as_u16()), message: error.to_string() }
}

async fn error_of(response: reqwest::Response) -> ProviderError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or(body);
    ProviderError { status: Some(status), message }
}

enum Feed {
    Openai(openai::Stream),
    Anthropic(anthropic::Stream),
}

impl Feed {
    fn feed(&mut self, data: &str) -> Result<Option<String>, String> {
        match self {
            Feed::Openai(s) => s.feed(data),
            Feed::Anthropic(s) => s.feed(data),
        }
    }

    fn finish(self) -> StepResult {
        match self {
            Feed::Openai(s) => s.finish(),
            Feed::Anthropic(s) => s.finish(),
        }
    }
}

pub async fn step(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    input: &StepInput<'_>,
    cancel: &CancellationToken,
    mut on_text: impl FnMut(&str),
) -> Result<StepResult, ProviderError> {
    let (url, body, mut feed) = match endpoint.kind {
        Kind::Openai | Kind::Ollama => (
            join_url(&endpoint.base_url, "chat/completions"),
            openai::body(input.system, input.turns, input.tools, input.model, true),
            Feed::Openai(openai::Stream::default()),
        ),
        Kind::Anthropic => (
            join_url(&anthropic_base(&endpoint.base_url), "messages"),
            anthropic::body(&anthropic::BodyInput {
                system: input.system,
                turns: input.turns,
                tools: input.tools,
                model: input.model,
                effort: input.effort,
                max_tokens: input.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
                stream: true,
            }),
            Feed::Anthropic(anthropic::Stream::default()),
        ),
    };
    let request = with_auth(http.post(url).json(&body), endpoint);
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(ProviderError { status: None, message: "aborted".into() }),
        sent = request.send() => sent.map_err(net_error)?,
    };
    if !response.status().is_success() {
        return Err(error_of(response).await);
    }
    let mut bytes = response.bytes_stream();
    let mut buf = SseBuf::default();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(ProviderError { status: None, message: "aborted".into() }),
            next = bytes.next() => next,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(net_error)?;
        for event in buf.push(&chunk) {
            if event.data.is_empty() {
                continue;
            }
            let delta = feed.feed(&event.data).map_err(|message| ProviderError { status: None, message })?;
            if let Some(text) = delta {
                on_text(&text);
            }
        }
    }
    if let Some(event) = buf.finish().filter(|e| !e.data.is_empty()) {
        let delta = feed.feed(&event.data).map_err(|message| ProviderError { status: None, message })?;
        if let Some(text) = delta {
            on_text(&text);
        }
    }
    Ok(feed.finish())
}

pub async fn models(http: &reqwest::Client, endpoint: &Endpoint) -> Result<Vec<Model>, ProviderError> {
    let body = models_body(http, endpoint).await?;
    Ok(match endpoint.kind {
        Kind::Openai | Kind::Ollama => openai::models_of(&body),
        Kind::Anthropic => anthropic::models_of(&body),
    })
}

pub async fn models_body(http: &reqwest::Client, endpoint: &Endpoint) -> Result<Value, ProviderError> {
    let url = match endpoint.kind {
        Kind::Openai | Kind::Ollama => join_url(&endpoint.base_url, "models"),
        Kind::Anthropic => join_url(&anthropic_base(&endpoint.base_url), "models?limit=100"),
    };
    let request = with_auth(http.get(url), endpoint).timeout(Duration::from_secs(MODELS_SECS));
    let response = request.send().await.map_err(net_error)?;
    if !response.status().is_success() {
        return Err(error_of(response).await);
    }
    response.json().await.map_err(net_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cross_host_redirect_keeps_the_key_home() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let other_port = other.local_addr().unwrap().port();
        let seen = tokio::spawn(async move {
            let Ok(Ok((mut s, _))) = tokio::time::timeout(Duration::from_millis(1500), other.accept()).await else { return String::new() };
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap_or(0);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok").await;
            String::from_utf8_lossy(&buf[..n]).to_lowercase()
        });
        let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = first.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = first.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = s.read(&mut buf).await;
            let reply = format!("HTTP/1.1 307 Temporary Redirect\r\nlocation: http://localhost:{other_port}/v1\r\ncontent-length: 0\r\n\r\n");
            let _ = s.write_all(reply.as_bytes()).await;
        });
        let res = client().get(format!("http://127.0.0.1:{port}/v1")).header("x-api-key", "secret-key-123").send().await.unwrap();
        assert_eq!(res.status().as_u16(), 307);
        assert!(!seen.await.unwrap().contains("secret-key-123"));
    }

    #[test]
    fn plain_errors() {
        assert_eq!(plain_error(Some(401), ""), "The key was refused (401).");
        assert_eq!(plain_error(None, "operation timed out"), "The server did not answer in time.");
        assert!(plain_error(None, &"x".repeat(300)).ends_with('…'));
    }

    #[test]
    fn builds_urls() {
        assert_eq!(join_url("https://a/v1/", "/models"), "https://a/v1/models");
        assert_eq!(anthropic_base("https://api.anthropic.com"), "https://api.anthropic.com/v1");
        assert_eq!(anthropic_base("http://localhost:1/v1/"), "http://localhost:1/v1");
    }

    #[test]
    fn ollama_kind() {
        assert_eq!(Kind::parse("ollama"), Some(Kind::Ollama));
        assert_eq!(Kind::Ollama.word(), "ollama");
        assert_eq!(Kind::Ollama.title(), "Ollama");
        assert_eq!(Kind::Ollama.default_url(), "http://127.0.0.1:11434/v1");
        assert_eq!(serde_json::to_value(Kind::Ollama).unwrap(), "ollama");
        for kind in [Kind::Openai, Kind::Anthropic, Kind::Ollama] {
            assert_eq!(Kind::parse(kind.word()), Some(kind));
        }
    }

    #[test]
    fn no_auth_header_without_a_key() {
        let http = reqwest::Client::new();
        let bare = Endpoint { kind: Kind::Ollama, base_url: "http://127.0.0.1:1/v1".into(), api_key: String::new() };
        let request = with_auth(http.get("http://127.0.0.1:1/v1/models"), &bare).build().unwrap();
        assert!(request.headers().get("authorization").is_none());
        let keyed = Endpoint { api_key: "some-key".into(), ..bare };
        let request = with_auth(http.get("http://127.0.0.1:1/v1/models"), &keyed).build().unwrap();
        assert_eq!(request.headers().get("authorization").unwrap(), "Bearer some-key");
    }

    #[test]
    fn debug_hides_key() {
        let endpoint = Endpoint { kind: Kind::Openai, base_url: "u".into(), api_key: "secret-value".into() };
        assert!(!format!("{endpoint:?}").contains("secret-value"));
    }
}
