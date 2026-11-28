use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
pub const ID_SUFFIX_LEN: usize = 6;
pub const ID_NUMBER_LEN: usize = 12;
pub const ID_SLUG_LEN: usize = 24;
pub const SPACE_SLUG_LABEL: &str = "nixa.space-slug";
pub const TERM_ID_PATTERN: &str = r"(?:[a-z0-9]+(?:-[a-z0-9]+)+-(?:[1-9][0-9]{0,11}|[a-z0-9]{6})|m[0-9a-z]+)";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TermId(String);

impl TermId {
    pub fn from_number(n: u64) -> TermId {
        TermId(format!("m{}", to_base36(n)))
    }

    pub fn number(&self) -> u64 {
        self.0.strip_prefix('m').and_then(parse_base36).unwrap_or(0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn own_number(&self) -> Option<u64> {
        let (_, suffix) = self.0.rsplit_once('-')?;
        number_suffix(suffix).then(|| suffix.parse().ok()).flatten()
    }
}

pub fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                if out.len() + 1 >= ID_SLUG_LEN {
                    break;
                }
                out.push('-');
            }
            if out.len() == ID_SLUG_LEN {
                break;
            }
            out.push(byte.to_ascii_lowercase() as char);
            gap = false;
        } else {
            gap = true;
        }
    }
    out
}

fn readable(s: &str) -> bool {
    let Some((prefix, suffix)) = s.rsplit_once('-') else {
        return false;
    };
    let alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    prefix.len() <= 2 * ID_SLUG_LEN + 1
        && prefix.contains('-')
        && prefix
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(alphanumeric))
        && (number_suffix(suffix) || (suffix.len() == ID_SUFFIX_LEN && suffix.bytes().all(alphanumeric)))
}

fn number_suffix(s: &str) -> bool {
    (1..=ID_NUMBER_LEN).contains(&s.len()) && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

pub fn parse_base36(s: &str) -> Option<u64> {
    if s.is_empty() || s.starts_with('0') {
        return None;
    }
    let mut n: u64 = 0;
    for b in s.bytes() {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'z' => b - b'a' + 10,
            _ => return None,
        };
        n = n.checked_mul(36)?.checked_add(d as u64)?;
    }
    Some(n)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadTermId(pub String);

impl fmt::Display for BadTermId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bad terminal id {:?}, expected space-agent-number like nixa-claude-12, an older space-agent-xxxxxx id or a legacy id like m1",
            self.0
        )
    }
}

impl std::error::Error for BadTermId {}

impl FromStr for TermId {
    type Err = BadTermId;

    fn from_str(s: &str) -> Result<TermId, BadTermId> {
        if s.strip_prefix('m').and_then(parse_base36).is_some() || readable(s) {
            Ok(TermId(s.to_owned()))
        } else {
            Err(BadTermId(s.to_owned()))
        }
    }
}

impl TryFrom<String> for TermId {
    type Error = BadTermId;

    fn try_from(s: String) -> Result<TermId, BadTermId> {
        s.parse()
    }
}

impl From<TermId> for String {
    fn from(id: TermId) -> String {
        id.0
    }
}

impl fmt::Display for TermId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Ord for TermId {
    fn cmp(&self, other: &TermId) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}

impl PartialOrd for TermId {
    fn partial_cmp(&self, other: &TermId) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_ids_carry_their_own_number() {
        assert_eq!("nixa-claude-357".parse::<TermId>().unwrap().own_number(), Some(357));
        assert_eq!("blue-lab-codex-1".parse::<TermId>().unwrap().own_number(), Some(1));
        assert_eq!("nixa-claude-x7k2r8".parse::<TermId>().unwrap().own_number(), None);
        assert_eq!("nixa-claude-000001".parse::<TermId>().unwrap().own_number(), None);
        assert_eq!(TermId::from_number(5).own_number(), None);
    }

    #[test]
    fn ids_count_in_base36() {
        assert_eq!(TermId::from_number(1).as_str(), "m1");
        assert_eq!(TermId::from_number(10).as_str(), "ma");
        assert_eq!(TermId::from_number(36).as_str(), "m10");
        assert_eq!(TermId::from_number(36).number(), 36);
    }

    #[test]
    fn bad_ids_are_rejected() {
        for s in [
            "",
            "m",
            "m0",
            "m01",
            "x1",
            "mA",
            "m-1",
            "repo-000001",
            "repo--agent-000001",
            "repo-agent-ABC123",
            "../repo-agent-000001",
            "repo-agent-12345a7",
            "repo-agent-012",
            "repo-agent-1234567890123",
            "repo-12",
        ] {
            assert!(s.parse::<TermId>().is_err(), "{s}");
        }
        assert!(serde_json::from_str::<TermId>("\"q9\"").is_err());
    }

    #[test]
    fn readable_ids_round_trip_and_keep_legacy_ids_valid() {
        let id: TermId = "blue-lab-claude-0x7k2z".parse().unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<TermId>(&json).unwrap(), id);
        assert_eq!("m10".parse::<TermId>().unwrap().number(), 36);
        for s in ["nixa-claude-12", "blue-lab-codex-1", "nixa-claude-x7k2r8", "nixa-claude-000001", "nixa-claude-123456789012"] {
            assert_eq!(s.parse::<TermId>().unwrap().as_str(), s);
        }
        assert_eq!(slug("  Blue__Lab / Build! "), "blue-lab-build");
        let long = slug("12345678901234567890123 next-word");
        assert!(long.len() <= ID_SLUG_LEN && !long.ends_with('-'));
    }

    #[test]
    fn ids_sort_by_number() {
        let mut ids: Vec<TermId> = [36, 2, 10, 1]
            .into_iter()
            .map(TermId::from_number)
            .collect();
        ids.sort();
        let nums: Vec<u64> = ids.iter().map(TermId::number).collect();
        assert_eq!(nums, vec![1, 2, 10, 36]);
    }
}
