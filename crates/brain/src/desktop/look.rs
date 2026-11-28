use crate::provider::{Endpoint, Kind, with_auth};
use nixa_desktop::picture::Picture;
use serde_json::{Value, json};
use std::time::Duration;

pub const LOOK_SECS: u64 = 90;
pub const ANSWER_TOKENS: u32 = 1024;
pub const CONNECT_SECS: u64 = 10;
pub const REDIRECT_TEXT: &str = "The vision server tried to send the pictures to another address, so they were not sent.";

pub fn local_client() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(CONNECT_SECS)).build().unwrap_or_default()
}

fn join(base: &str, tail: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), tail)
}

fn anthropic_base(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1") { trimmed.to_string() } else { format!("{trimmed}/v1") }
}

pub fn request_of(endpoint: &Endpoint, model: &str, system: &str, question: &str, pictures: &[Picture]) -> (String, Value) {
    match endpoint.kind {
        Kind::Anthropic => {
            let mut content: Vec<Value> = pictures.iter().map(|p| json!({"type": "image", "source": {"type": "base64", "media_type": p.media_type, "data": p.data}})).collect();
            content.push(json!({"type": "text", "text": question}));
            (
            join(&anthropic_base(&endpoint.base_url), "messages"),
            json!({
                "model": model,
                "max_tokens": ANSWER_TOKENS,
                "system": system,
                "messages": [{"role": "user", "content": content}],
            }),
            )
        }
        Kind::Openai | Kind::Ollama => {
            let mut content = vec![json!({"type": "text", "text": question})];
            content.extend(pictures.iter().map(|p| json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", p.media_type, p.data)}})));
            (
            join(&endpoint.base_url, "chat/completions"),
            json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": content},
                ],
            }),
            )
        }
    }
}

pub fn answer_of(kind: Kind, body: &Value) -> String {
    match kind {
        Kind::Anthropic => body
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| parts.iter().filter(|p| p.get("type").and_then(Value::as_str) == Some("text")).filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default(),
        Kind::Openai | Kind::Ollama => {
            let content = body.pointer("/choices/0/message/content");
            match content {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            }
        }
    }
}

pub async fn look(http: &reqwest::Client, endpoint: &Endpoint, model: &str, system: &str, question: &str, pictures: &[Picture]) -> Result<String, String> {
    let (url, body) = request_of(endpoint, model, system, question, pictures);
    let request = with_auth(http.post(url).json(&body), endpoint).timeout(Duration::from_secs(LOOK_SECS));
    let response = request.send().await.map_err(|e| crate::provider::plain_error(e.status().map(|s| s.as_u16()), &e.to_string()))?;
    let status = response.status().as_u16();
    if response.status().is_redirection() {
        return Err(REDIRECT_TEXT.into());
    }
    let text = response.text().await.map_err(|e| crate::provider::plain_error(None, &e.to_string()))?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = parsed.pointer("/error/message").and_then(Value::as_str).map(str::to_string).unwrap_or(text);
        return Err(crate::provider::plain_error(Some(status), &message));
    }
    Ok(answer_of(endpoint.kind, &parsed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn a_local_look_does_not_follow_a_redirect() {
        let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote_addr = remote.local_addr().unwrap();
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = local.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = local.accept().await.unwrap();
            let mut buf = vec![0u8; 65536];
            let _ = sock.read(&mut buf).await;
            let reply = format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{remote_addr}/v1/chat/completions\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            sock.write_all(reply.as_bytes()).await.unwrap();
        });
        let picture = Picture { media_type: "image/jpeg", data: "AAAA".into(), bytes: 3 };
        let endpoint = Endpoint { kind: Kind::Openai, base_url: format!("http://{local_addr}/v1"), api_key: String::new() };
        let result = look(&local_client(), &endpoint, "m", "sys", "What?", std::slice::from_ref(&picture)).await;
        assert_eq!(result, Err(REDIRECT_TEXT.to_string()));
        let reached = tokio::time::timeout(Duration::from_millis(300), remote.accept()).await;
        assert!(reached.is_err(), "the redirect target must not be reached");
    }
}
