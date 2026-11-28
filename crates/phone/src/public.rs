use anyhow::{Result, anyhow, bail};
use regex::Regex;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

const PRIVATE_SUFFIXES: [&str; 7] = ["localhost", "local", "internal", "lan", "home", "test", "invalid"];

pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || b == 0 || b == 88))
                || (a == 198 && (b == 18 || b == 19 || b == 51))
                || (a == 203 && b == 0))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            (0x2000..=0x3fff).contains(&s[0])
                && s[0] != 0x2002
                && s[0] != 0x3fff
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}

fn bare(host: &str) -> String {
    host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase()
}

pub fn check_host(host: &str) -> Result<()> {
    let host = bare(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if public_address(ip) {
            return Ok(());
        }
        bail!("Private or internal addresses are blocked.");
    }
    let good_chars = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !good_chars || !host.contains('.') || host.starts_with('.') || host.contains("..") {
        bail!("Only public HTTP or HTTPS pages on standard ports are allowed.");
    }
    let last = host.rsplit('.').next().unwrap_or("");
    if PRIVATE_SUFFIXES.contains(&last) || host.split('.').all(|label| label.bytes().all(|b| b.is_ascii_digit())) {
        bail!("Private or internal addresses are blocked.");
    }
    Ok(())
}

fn private_words() -> &'static Regex {
    static WORDS: OnceLock<Regex> = OnceLock::new();
    WORDS.get_or_init(|| {
        Regex::new(r"(?i)[\r\n{}<>\x60]|/Users/|/home/|file:|\b(?:my|our|private|personal|client|customer|password|passwd|secret|token|api[_ -]?key|bearer|cookie|confidential|internal|client data|private key)\b|\b\d{4}[ -]?\d{4}[ -]?\d{4}[ -]?\d{4}\b")
            .expect("private word pattern")
    })
}

pub fn public_text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().count() > 240 || private_words().is_match(value) {
        bail!("Only a short public question or public text is allowed. Private data, code and credentials stay here.");
    }
    Ok(())
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hexpair = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hexpair) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn public_lookup(host: &str, port: u16) -> Result<SocketAddr> {
    let host = bare(host);
    let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|_| anyhow!("Private or internal addresses are blocked."))?.collect();
    if found.is_empty() || found.iter().any(|a| !public_address(a.ip())) {
        bail!("Private or internal addresses are blocked.");
    }
    Ok(found[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn only_public_addresses_pass() {
        for good in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(public_address(ip(good)), "{good}");
        }
        for bad in [
            "0.1.2.3", "10.0.0.1", "127.0.0.1", "100.64.0.1", "169.254.1.1", "172.16.0.1", "192.168.1.1", "192.0.0.1",
            "198.18.0.1", "203.0.113.5", "224.0.0.1", "::1", "fe80::1", "fc00::1", "::ffff:8.8.8.8", "2002::1", "2001:db8::1",
            "2001:10::1",
        ] {
            assert!(!public_address(ip(bad)), "{bad}");
        }
    }

    #[test]
    fn hosts_are_checked() {
        assert!(check_host("example.com").is_ok());
        assert!(check_host("[2606:4700::1111]").is_ok());
        assert!(check_host("localhost").is_err());
        assert!(check_host("printer.local").is_err());
        assert!(check_host("nas.lan.").is_err());
        assert!(check_host("192.168.1.1").is_err());
        assert!(check_host("intranet").is_err());
        assert!(check_host("a..b.com").is_err());
        assert!(check_host("bad_host.com").is_err());
    }

    #[test]
    fn private_text_is_blocked() {
        assert!(public_text("/search?q=rust+async").is_ok());
        assert!(public_text("/Users/someone/notes").is_err());
        assert!(public_text("/?q=my password").is_err());
        assert!(public_text("/?card=4111 1111 1111 1111").is_err());
        assert!(public_text(&format!("/{}", "a".repeat(300))).is_err());
        assert!(public_text("/a\x60b").is_err());
        assert_eq!(percent_decode("/a%20b%zz%2"), "/a b%zz%2");
    }

    #[tokio::test]
    async fn lookups_of_local_names_are_blocked() {
        assert!(public_lookup("localhost", 80).await.is_err());
        assert!(public_lookup("127.0.0.1", 80).await.is_err());
    }
}
