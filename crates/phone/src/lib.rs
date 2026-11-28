mod audio;
pub mod cert;
mod door;
#[cfg(test)]
mod door_tests;
pub mod files;
pub mod net;
pub mod ports;
pub mod public;
pub mod public_proxy;
pub mod qr;
mod routes;
pub mod secret;
pub mod sink;
mod socket;
mod tls;
pub mod wire;

pub use cert::DoorNames;
pub use door::{AddressFn, CameraPhone, Deps, Door, NamesFn, NowFn, PhoneCheck, PhoneInfo, PhoneRow, SnapshotFn};
pub use ports::{PHONE_PORT, SETUP_PORT};
pub use public_proxy::{PublicProxy, start_public_proxy};
pub use sink::{Pcm, PhoneJob, PhoneSink, SayResult, Synth};
pub use wire::{Frame, Lens, PhoneCommand, PhoneEvent, PlayedHow};

pub(crate) fn log(area: &str, message: &str) {
    eprintln!("[{}] {area}: {}", now_ms(), hide_secrets(message));
}

static SECRETS: std::sync::LazyLock<Vec<(regex::Regex, &'static str)>> = std::sync::LazyLock::new(|| {
    [
        (r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----(?s:.*?)(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)", "<hidden>"),
        (r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s:@/]+:[^\s@/]+@", "${1}<hidden>@"),
        (r"(?i)\b(authorization|cookie|x-api-key|api-key)(\s*[:=]\s*)[^\r\n]+", "${1}${2}<hidden>"),
        (r"(/k/)[A-Za-z0-9_-]{8,}", "${1}<hidden>"),
        (r#"\b([A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD)[A-Z0-9_]*\s*[=:]\s*)["']?[^\s"']{6,}["']?"#, "${1}<hidden>"),
        (r"\b(?:sk|rk|pk)-(?:ant-|proj-|or-|svcacct-)?[A-Za-z0-9_-]{16,}|\bxai-[A-Za-z0-9_-]{16,}|\bAIza[0-9A-Za-z_-]{30,}|\bgh[pousr]_[A-Za-z0-9]{20,}|\bgithub_pat_[A-Za-z0-9_]{20,}|\bxox[abprs]-[A-Za-z0-9-]{10,}|\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", "<hidden>"),
        (r"\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{12,}", "${1} <hidden>"),
    ]
    .into_iter()
    .filter_map(|(pattern, to)| regex::Regex::new(pattern).ok().map(|re| (re, to)))
    .collect()
});

#[cfg(test)]
#[test]
fn log_lines_hide_secrets() {
    let out = hide_secrets("Authorization: Bearer abcdefghijklmnop and http://1.2.3.4:9/k/abcdefgh1234");
    assert!(!out.contains("abcdefghijklmnop") && !out.contains("abcdefgh1234"), "{out}");
}

pub(crate) fn hide_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for (pattern, to) in SECRETS.iter() {
        out = pattern.replace_all(&out, *to).into_owned();
    }
    out
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
