use regex::Regex;
use reqwest::Url;
use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;

pub const MAX_TEXT: usize = 240;
pub const PUBLIC_ONLY: &str = "Only a short public question or public text is allowed. Private data, code and credentials stay here.";
pub const PUBLIC_PAGE: &str = "Only public HTTP or HTTPS pages on standard ports are allowed.";
pub const PRIVATE_ADDRESS: &str = "Private or internal addresses are blocked.";
pub const BLOCKED_ENDS: [&str; 7] = ["localhost", "local", "internal", "lan", "home", "test", "invalid"];

static PRIVATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[\r\n{}<>`]|/Users/|/home/|file:|\b(?:my|our|private|personal|client|customer|password|passwd|secret|token|api[_ -]?key|bearer|cookie|confidential|internal|client data|private key)\b|\b\d{4}[ -]?\d{4}[ -]?\d{4}[ -]?\d{4}\b").expect("public text pattern")
});

pub fn public_text(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > MAX_TEXT || PRIVATE.is_match(value) || crate::privacy::redact::redact(value) != value {
        return Err(PUBLIC_ONLY.into());
    }
    Ok(())
}

pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && matches!(c, 0 | 2))
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && matches!(b, 18 | 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            (0x2000..=0x3fff).contains(&s[0]) && s[0] != 0x2002 && s[0] != 0x3fff && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}

fn bare_host(host: &str) -> &str {
    host.trim_start_matches('[').trim_end_matches(']')
}

pub fn blocked_name(host: &str) -> bool {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    BLOCKED_ENDS.iter().any(|end| name == *end || name.ends_with(&format!(".{end}")))
}

fn hex(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|d| d as u8)
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn public_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| PUBLIC_PAGE.to_string())?;
    let host = url.host_str().unwrap_or_default().to_string();
    let port_ok = url.port().is_none_or(|p| p == 80 || p == 443);
    let ip_bad = bare_host(&host).parse::<IpAddr>().is_ok_and(|ip| !public_address(ip));
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() || !port_ok || !host.contains('.') || ip_bad {
        return Err(PUBLIC_PAGE.into());
    }
    if blocked_name(&host) {
        return Err(PRIVATE_ADDRESS.into());
    }
    let mut rest = url.path().to_string();
    if let Some(query) = url.query() {
        rest.push('?');
        rest.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        rest.push('#');
        rest.push_str(fragment);
    }
    public_text(&percent_decode(&rest))?;
    Ok(url)
}

pub async fn public_lookup(host: &str, port: u16) -> Result<SocketAddr, String> {
    let found: Vec<SocketAddr> = tokio::net::lookup_host((bare_host(host), port)).await.map_err(|_| PRIVATE_ADDRESS.to_string())?.collect();
    match found.first() {
        Some(first) if found.iter().all(|a| public_address(a.ip())) => Ok(*first),
        _ => Err(PRIVATE_ADDRESS.into()),
    }
}
