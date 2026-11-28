use regex::{Captures, Regex};
use std::sync::OnceLock;

pub const SECRET_MIN: usize = 8;
const HIDDEN: &str = "<hidden>";

fn secret_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:^|_)(?:API_?KEY|KEY|KEYS|TOKEN|SECRET|SECRETS|PASSWORD|PASSWD|PASSPHRASE|CREDENTIALS?|AUTH|DSN|PRIVATE_KEY|ACCESS_KEY)(?:_|$)")
            .expect("secret name regex")
    })
}

fn plain_value() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:true|false|yes|no|on|off|null|undefined|\d+(?:\.\d+)?)$")
            .expect("plain value regex")
    })
}

fn env_assign() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"\b([A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASSPHRASE|CREDENTIALS?|DSN)[A-Z0-9_]*)(\s*[=:]\s*)(["']?)([^\s"'`]{6,})"#)
            .expect("env assign regex")
    })
}

fn shapes() -> &'static [(Regex, &'static str)] {
    static LIST: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    LIST.get_or_init(|| {
        let raw: [(&str, &str); 13] = [
            (r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)", "<private key hidden>"),
            (r"(?i)\b((?:proxy-)?authorization|cookie|set-cookie|x-api-key|api-key|x-auth-token)(\s*:\s*)[^\r\n]+", "${1}${2}<hidden>"),
            (r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s:@/]+:[^\s@/]+@", "${1}<hidden>@"),
            (r#"(?i)("[A-Za-z_]*(?:key|token|secret|password|passwd|credential)[A-Za-z_]*"\s*:\s*)"[^"]{6,}""#, "${1}\"<hidden>\""),
            (r"\b(?:sk|rk|pk)-(?:ant-|proj-|or-|svcacct-)?[A-Za-z0-9_-]{16,}", "<key hidden>"),
            (r"\bxai-[A-Za-z0-9_-]{16,}", "<key hidden>"),
            (r"\bAIza[0-9A-Za-z_-]{30,}", "<key hidden>"),
            (r"\bgh[pousr]_[A-Za-z0-9]{20,}", "<key hidden>"),
            (r"\bgithub_pat_[A-Za-z0-9_]{20,}", "<key hidden>"),
            (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", "<key hidden>"),
            (r"\bAKIA[0-9A-Z]{16}\b", "<key hidden>"),
            (r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}", "<token hidden>"),
            (r"\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{12,}", "${1} <token hidden>"),
        ];
        raw.iter()
            .map(|(pattern, to)| (Regex::new(pattern).expect("secret shape regex"), *to))
            .collect()
    })
}

pub fn known_secrets(env: impl Iterator<Item = (String, String)>) -> Vec<String> {
    let mut found: Vec<String> = env
        .filter(|(name, _)| secret_name().is_match(name))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| {
            value.chars().count() >= SECRET_MIN
                && !plain_value().is_match(value)
                && !value.starts_with(['/', '~', '.'])
        })
        .collect();
    found.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    found.dedup();
    found
}

fn keep_env_value(value: &str) -> bool {
    ["process.env", "os.environ", "import.meta", "$"]
        .iter()
        .any(|start| value.starts_with(start))
}

pub fn hide(text: &str, known: &[String]) -> String {
    let mut out = text.to_string();
    for value in known {
        if value.chars().count() >= SECRET_MIN && out.contains(value.as_str()) {
            out = out.replace(value.as_str(), HIDDEN);
        }
    }
    for (pattern, to) in shapes().iter().take(3) {
        out = pattern.replace_all(&out, *to).into_owned();
    }
    out = env_assign()
        .replace_all(&out, |caps: &Captures| {
            if keep_env_value(&caps[4]) {
                caps[0].to_string()
            } else {
                format!("{}{}{}{HIDDEN}", &caps[1], &caps[2], &caps[3])
            }
        })
        .into_owned();
    for (pattern, to) in shapes().iter().skip(3) {
        out = pattern.replace_all(&out, *to).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(prefix: &str, len: usize) -> String {
        format!("{prefix}{}", "x".repeat(len))
    }

    #[test]
    fn hides_known_values() {
        let value = fake("v", 20);
        let known = known_secrets(
            vec![
                ("SERVICE_TOKEN".to_string(), value.clone()),
                ("HOME".to_string(), "/Users/someone".to_string()),
                ("DEBUG_AUTH".to_string(), "true".to_string()),
            ]
            .into_iter(),
        );
        assert_eq!(known, vec![value.clone()]);
        assert_eq!(hide(&format!("got {value} here"), &known), "got <hidden> here");
    }

    #[test]
    fn hides_shapes() {
        let key = fake(&["sk", "-"].concat(), 24);
        assert_eq!(hide(&format!("key {key}"), &[]), "key <key hidden>");
        let line = format!("{}={}", "SERVICE_TOKEN", fake("q", 10));
        assert_eq!(hide(&line, &[]), "SERVICE_TOKEN=<hidden>");
        assert_eq!(hide("SERVICE_TOKEN=$OTHER_NAME", &[]), "SERVICE_TOKEN=$OTHER_NAME");
        let auth = format!("Authorization: {} {}", "Bearer", fake("t", 20));
        assert_eq!(hide(&auth, &[]), "Authorization: <hidden>");
        let url = format!("https://user:{}@example.com/x", fake("p", 8));
        assert_eq!(hide(&url, &[]), "https://<hidden>@example.com/x");
        assert_eq!(hide("plain words stay", &[]), "plain words stay");
    }
}
