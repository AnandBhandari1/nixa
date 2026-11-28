use super::Plan;
use crate::app::App;
use crate::connectors::{builtin::CAMERA, command};
use reqwest::Url;
use std::io::Write;
use std::net::IpAddr;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 4 * 1024 * 1024;

fn local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => ip.is_unique_local(),
    }
}

fn endpoint(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "The saved camera URL is invalid.")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Save an HTTP or HTTPS snapshot URL without embedded credentials or a fragment.".into());
    }
    Ok(url)
}

fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\xff\xd8\xff") && bytes.ends_with(b"\xff\xd9") { Some("jpg") }
    else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.ends_with(b"IEND\xaeB\x60\x82") { Some("png") }
    else { None }
}

pub(super) async fn run(app: &App, plan: &Plan) -> Result<String, String> {
    if super::super::private_blocked(app) { return Err(super::super::call::PRIVATE_BLOCKED.into()); }
    if !plan.input.is_empty() { return Err("Camera actions do not take input.".into()); }
    let raw = plan.secrets.get(CAMERA).ok_or("Set the camera snapshot URL in Settings, Connectors.")?;
    tokio::time::timeout(TIMEOUT, fetch(app, endpoint(raw)?, plan.action().name == "snapshot"))
        .await.map_err(|_| "The camera did not respond in time.".to_string())?
}

async fn fetch(app: &App, url: Url, snapshot: bool) -> Result<String, String> {
    let host = url.host_str().ok_or("The saved camera URL has no host.")?.trim_matches(['[', ']']);
    let port = url.port_or_known_default().ok_or("The saved camera URL has no port.")?;
    let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await
        .map_err(|_| "The saved camera host could not be resolved.")?.collect();
    if addresses.is_empty() || !addresses.iter().all(|address| {
        local(address.ip()) || (url.scheme() == "https" && crate::web::safety::public_address(address.ip()))
    }) { return Err("Camera URLs must resolve to a private network address or a public HTTPS endpoint.".into()); }
    let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses).timeout(TIMEOUT).build().map_err(|_| "Could not prepare the camera request.")?;
    let mut response = client.get(url).send().await.map_err(|_| "The saved camera endpoint did not answer.")?;
    if !response.status().is_success() { return Err(format!("The camera returned HTTP {}.", response.status().as_u16())); }
    if !snapshot { return Ok("The saved camera endpoint responded successfully.".into()); }
    if response.content_length().is_some_and(|size| size > MAX_BYTES as u64) { return Err("The camera frame exceeds 4 MB.".into()); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "The camera frame could not be read.")? {
        if chunk.len() > MAX_BYTES - bytes.len() { return Err("The camera frame exceeds 4 MB.".into()); }
        bytes.extend_from_slice(&chunk);
    }
    let ext = image_kind(&bytes).ok_or("The saved URL did not return one complete JPEG or PNG frame.")?;
    let dir = command::private_folder(&app.connectors.root, CAMERA)?;
    let frame = dir.join(format!("{}.{}", crate::cli_brains::new_token(), ext));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&frame)
        .map_err(|_| "Could not create a private camera frame.")?;
    if file.write_all(&bytes).is_err() {
        let _ = std::fs::remove_file(&frame);
        return Err("Could not save the camera frame.".into());
    }
    Ok(format!("Image file: {}\nBytes: {}", frame.display(), bytes.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_snapshot_urls_and_complete_images() {
        assert!(endpoint("https://example.invalid/frame").is_ok());
        assert!(endpoint("file:///tmp/frame").is_err());
        assert!(endpoint("https://user:secret@example.invalid/frame").is_err());
        assert!(endpoint("rtsp://example.invalid/frame").is_err());
        assert!(!local(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
        assert_eq!(image_kind(b"\xff\xd8\xffframe\xff\xd9"), Some("jpg"));
        assert_eq!(image_kind(b"\xff\xd8\xffunfinished"), None);
    }
}
