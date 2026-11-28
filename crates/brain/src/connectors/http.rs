use super::manifest::{Permission, Request, SLOT, slots, whole_slot};
use reqwest::{Method, StatusCode, Url};
use serde_json::{Map, Value};
use std::collections::HashMap;

pub const MAX_BODY: usize = 2 * 1024 * 1024;
pub const MAX_HOPS: usize = 3;
pub const AGENT: &str = "Nixa connector";

pub struct Net {
    pub client: reqwest::Client,
    pub plain_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub method: String,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(AGENT)
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

pub fn live() -> Net {
    Net { client: client(), plain_port: None }
}

pub fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn lookup(secret: bool, name: &str, input: &Map<String, Value>, secrets: &HashMap<String, String>) -> Option<String> {
    if secret { secrets.get(name).cloned() } else { input.get(name).map(value_text) }
}

pub fn fill(template: &str, input: &Map<String, Value>, secrets: &HashMap<String, String>, url: bool) -> Result<Option<String>, String> {
    if let Some(c) = SLOT.captures(template).filter(|c| c.get(0).is_some_and(|m| m.as_str() == template)) {
        let found = lookup(c.get(1).is_some(), &c[2], input, secrets);
        if found.is_none() && url {
            return Err(format!("The address needs the input {}.", &c[2]));
        }
        return Ok(found.map(|v| if url { encode(&v) } else { v }));
    }
    let mut missing = None;
    let out = SLOT.replace_all(template, |c: &regex::Captures| match lookup(c.get(1).is_some(), &c[2], input, secrets) {
        Some(v) if url => encode(&v),
        Some(v) => v,
        None => {
            missing.get_or_insert_with(|| c[2].to_string());
            String::new()
        }
    });
    if url && let Some(name) = missing {
        return Err(format!("The address needs the input {name}."));
    }
    Ok(Some(out.into_owned()))
}

pub fn fill_value(value: &Value, input: &Map<String, Value>, secrets: &HashMap<String, String>) -> Option<Value> {
    match value {
        Value::String(s) => {
            if let Some(c) = SLOT.captures(s).filter(|c| c.get(0).is_some_and(|m| m.as_str() == s)) {
                return if c.get(1).is_some() { secrets.get(&c[2]).map(|v| Value::String(v.clone())) } else { input.get(&c[2]).cloned() };
            }
            fill(s, input, secrets, false).ok().flatten().map(Value::String)
        }
        Value::Array(items) => Some(Value::Array(items.iter().filter_map(|v| fill_value(v, input, secrets)).collect())),
        Value::Object(map) => Some(Value::Object(map.iter().filter_map(|(k, v)| fill_value(v, input, secrets).map(|v| (k.clone(), v))).collect())),
        other => Some(other.clone()),
    }
}

pub fn whole_address(template: &str, input: &Map<String, Value>) -> Option<Result<String, String>> {
    let name = whole_slot(template)?;
    Some(input.get(&name).map(value_text).ok_or_else(|| format!("The address needs the input {name}.")))
}

pub fn has_secrets(template: &str, secrets: &HashMap<String, String>) -> bool {
    slots(template).iter().all(|(secret, name)| !secret || secrets.contains_key(name))
}

pub fn build(request: &Request, input: &Map<String, Value>, secrets: &HashMap<String, String>) -> Result<Built, String> {
    let address = match whole_address(&request.url, input) {
        Some(found) => found?,
        None => fill(&request.url, input, secrets, true)?.unwrap_or_default(),
    };
    let mut url = Url::parse(&address).map_err(|_| "The address of this action is not a valid link.".to_string())?;
    let pairs: Vec<(String, String)> = request
        .query
        .iter()
        .filter_map(|(k, v)| fill(v, input, secrets, false).ok().flatten().map(|v| (k.clone(), v)))
        .collect();
    if !pairs.is_empty() {
        let mut query = url.query_pairs_mut();
        for (k, v) in &pairs {
            query.append_pair(k, v);
        }
    }
    let headers = request.headers.iter().filter(|(_, v)| has_secrets(v, secrets)).filter_map(|(k, v)| fill(v, input, secrets, false).ok().flatten().map(|v| (k.clone(), v))).collect();
    let body = request.body.as_ref().and_then(|b| fill_value(b, input, secrets));
    Ok(Built { method: request.method.clone(), url, headers, body })
}

pub fn allowed(url: &Url, hosts: &[String], permission: Permission, method: &str) -> Result<(), String> {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err("Only plain https addresses on the default port are allowed.".into());
    }
    let host = url.host_str().unwrap_or_default().to_lowercase();
    if !hosts.contains(&host) {
        return Err(format!("The host {host} is not in this connector's hosts list."));
    }
    if permission == Permission::Read && method != "GET" && method != "HEAD" {
        return Err("A read action can only use GET or HEAD.".into());
    }
    Ok(())
}

pub fn wire(url: &Url, net: &Net) -> Url {
    let mut out = url.clone();
    if let Some(port) = net.plain_port {
        let _ = out.set_scheme("http");
        let _ = out.set_port(Some(port));
    }
    out
}

pub fn logical(url: Url, net: &Net) -> Url {
    let mut out = url;
    if let Some(port) = net.plain_port
        && out.scheme() == "http"
        && out.port() == Some(port)
    {
        let _ = out.set_port(None);
        let _ = out.set_scheme("https");
    }
    out
}

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("The answer stopped early: {}", e.without_url()))? {
        data.extend_from_slice(&chunk);
        if data.len() > MAX_BODY {
            return Err(format!("The answer is bigger than {} MB.", MAX_BODY / 1024 / 1024));
        }
    }
    Ok(data)
}

pub fn pick(text: &str, pointer: Option<&str>) -> Result<String, String> {
    let Some(pointer) = pointer else { return Ok(text.to_string()) };
    let value: Value = serde_json::from_str(text).map_err(|_| "The answer is not JSON, so pick cannot be used.".to_string())?;
    let found = value.pointer(pointer).ok_or_else(|| format!("The answer has no value at {pointer}."))?;
    Ok(value_text(found))
}

pub async fn send(net: &Net, hosts: &[String], permission: Permission, built: Built, pointer: Option<&str>) -> Result<String, String> {
    let mut url = built.url;
    let mut method = built.method;
    let mut body = built.body;
    for hop in 0..=MAX_HOPS {
        allowed(&url, hosts, permission, &method)?;
        let verb = Method::from_bytes(method.as_bytes()).map_err(|_| "The method is not valid.".to_string())?;
        let mut request = net.client.request(verb, wire(&url, net));
        let mut typed = false;
        for (name, value) in &built.headers {
            typed |= name.eq_ignore_ascii_case("content-type");
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(data) = &body {
            if !typed {
                request = request.header("content-type", "application/json");
            }
            request = request.body(match data {
                Value::String(s) if typed => s.clone(),
                other => other.to_string(),
            });
        }
        let response = request.send().await.map_err(|e| format!("The service could not be reached: {}", e.without_url()))?;
        let status = response.status();
        let next = response.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
        if status.is_redirection() && let Some(next) = next {
            if hop == MAX_HOPS {
                return Err("Too many redirects.".into());
            }
            let joined = wire(&url, net).join(&next).map_err(|_| "The service sent a bad redirect.".to_string())?;
            url = logical(joined, net);
            if status != StatusCode::TEMPORARY_REDIRECT && status != StatusCode::PERMANENT_REDIRECT {
                if method != "HEAD" {
                    method = "GET".into();
                }
                body = None;
            }
            continue;
        }
        let data = read_body(response).await?;
        let text = String::from_utf8_lossy(&data).to_string();
        if !status.is_success() {
            return Err(format!("The service answered {}: {}", status.as_u16(), crate::util::clip_flat(&text, 300)));
        }
        if method == "HEAD" {
            return Ok(format!("Status {}", status.as_u16()));
        }
        return pick(&text, pointer);
    }
    Err("Too many redirects.".into())
}
