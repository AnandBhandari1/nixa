use super::{page, safety};
use reqwest::Url;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HOPS: usize = 5;
pub const TIMEOUT_SECS: u64 = 15;
pub const DEFAULT_LENGTH: usize = 8000;
pub const MAX_LENGTH: usize = 20000;
pub const AGENT: &str = "Nixa";

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: Option<String>,
    pub text: String,
}

#[derive(Debug)]
pub enum Step {
    Moved(Url),
    Done(Page),
}

pub const NO_CLIENT: &str = "The safe page reader did not start.";

pub fn client_for(host: &str, addr: SocketAddr) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .user_agent(AGENT);
    if host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>().is_err() {
        builder = builder.resolve(host, addr);
    }
    builder.build().map_err(|_| NO_CLIENT.to_string())
}

pub async fn get_once(client: &reqwest::Client, url: &Url) -> Result<Step, String> {
    let mut res = client
        .get(url.clone())
        .header(ACCEPT, "text/html, text/plain;q=0.9, */*;q=0.5")
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "The page did not answer in time.".to_string() } else { "The page could not be reached.".to_string() })?;
    if res.status().is_redirection() {
        let to = res.headers().get(LOCATION).and_then(|v| v.to_str().ok()).ok_or("The page moved but did not say where.")?;
        return url.join(to).map(Step::Moved).map_err(|_| safety::PUBLIC_PAGE.to_string());
    }
    if !res.status().is_success() {
        return Err(format!("The page answered {}.", res.status().as_u16()));
    }
    let kind = res.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    if !page::readable(&kind) {
        return Err(format!("The page is not text ({kind})."));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|_| "The page stopped while it was read.".to_string())? {
        let room = MAX_BYTES - body.len();
        if chunk.len() >= room {
            body.extend_from_slice(&chunk[..room]);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    let raw = String::from_utf8_lossy(&body).into_owned();
    let (title, text) = if page::is_html(&kind, &raw) { (page::title(&raw), page::html_text(&raw)) } else { (None, page::tidy(&raw)) };
    Ok(Step::Done(Page { url: url.to_string(), title, text }))
}

pub async fn fetch(raw: &str) -> Result<Page, String> {
    let mut url = safety::public_url(raw)?;
    for _ in 0..=MAX_HOPS {
        let host = url.host_str().ok_or(safety::PUBLIC_PAGE)?.to_string();
        let port = url.port_or_known_default().unwrap_or(443);
        let addr = safety::public_lookup(&host, port).await?;
        match get_once(&client_for(&host, addr)?, &url).await? {
            Step::Moved(next) => url = safety::public_url(next.as_str())?,
            Step::Done(page) => return Ok(page),
        }
    }
    Err("The page moved too many times.".into())
}
