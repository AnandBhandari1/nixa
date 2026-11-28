use anyhow::Result;
use std::fmt;
use std::str::FromStr;

#[derive(Debug)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

pub fn usage(msg: impl Into<String>) -> anyhow::Error {
    UsageError(msg.into()).into()
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub pos: Vec<String>,
    pub rest: Vec<String>,
    opts: Vec<(String, Option<String>)>,
}

fn is_option(a: &str) -> bool {
    a.len() > 1 && a.starts_with('-') && a[1..].parse::<i64>().is_err()
}

pub fn parse(argv: &[String], with_value: &[&str], stop_at_pos: bool) -> Result<Parsed> {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        i += 1;
        if a == "--" {
            p.rest.extend(argv[i..].iter().cloned());
            break;
        }
        if !is_option(a) {
            if stop_at_pos {
                p.rest.extend(argv[i - 1..].iter().cloned());
                break;
            }
            p.pos.push(a.clone());
            continue;
        }
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        if with_value.contains(&name.as_str()) {
            let value = match inline {
                Some(v) => v,
                None => {
                    let v = argv
                        .get(i)
                        .cloned()
                        .ok_or_else(|| usage(format!("{name} needs a value")))?;
                    i += 1;
                    v
                }
            };
            p.opts.push((name, Some(value)));
        } else if inline.is_some() {
            return Err(usage(format!("{name} takes no value")));
        } else {
            p.opts.push((name, None));
        }
    }
    Ok(p)
}

impl Parsed {
    pub fn flag(&mut self, names: &[&str]) -> bool {
        let before = self.opts.len();
        self.opts
            .retain(|(n, v)| !(v.is_none() && names.contains(&n.as_str())));
        before != self.opts.len()
    }

    pub fn values(&mut self, names: &[&str]) -> Vec<String> {
        let mut found = Vec::new();
        self.opts.retain(|(n, v)| {
            if names.contains(&n.as_str()) {
                if let Some(v) = v {
                    found.push(v.clone());
                }
                false
            } else {
                true
            }
        });
        found
    }

    pub fn value(&mut self, names: &[&str]) -> Option<String> {
        self.values(names).pop()
    }

    pub fn finish(&self, max_pos: usize, rest_ok: bool) -> Result<()> {
        if let Some((name, _)) = self.opts.first() {
            return Err(usage(format!("unknown option {name}")));
        }
        if self.pos.len() > max_pos {
            return Err(usage(format!("unexpected argument {:?}", self.pos[max_pos])));
        }
        if !rest_ok && !self.rest.is_empty() {
            return Err(usage(format!("unexpected argument {:?}", self.rest[0])));
        }
        Ok(())
    }
}

pub fn number<T: FromStr>(v: &str, name: &str) -> Result<T> {
    v.trim()
        .parse::<T>()
        .map_err(|_| usage(format!("{name} needs a number, got {v:?}")))
}

pub fn duration_ms(v: &str) -> Result<u64> {
    let t = v.trim();
    let (num, mult) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1_000)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3_600_000)
    } else {
        (t, 1)
    };
    let n: u64 = num
        .parse()
        .map_err(|_| usage(format!("bad time {v:?}, use for example 500, 500ms, 30s, 5m or 1h")))?;
    n.checked_mul(mult).ok_or_else(|| usage(format!("time {v:?} is too large")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_values_and_rest() {
        let mut p = parse(&strs(&["m1", "--timeout", "5s", "-s", "hi", "--", "-x"]), &["--timeout"], false).unwrap();
        assert!(p.flag(&["-s"]));
        assert_eq!(p.value(&["--timeout"]).as_deref(), Some("5s"));
        assert_eq!(p.pos, strs(&["m1", "hi"]));
        assert_eq!(p.rest, strs(&["-x"]));
        p.finish(2, true).unwrap();
        assert!(p.finish(1, true).is_err());
        assert!(p.finish(2, false).is_err());
    }

    #[test]
    fn inline_values_and_unknown() {
        let mut p = parse(&strs(&["--name=a b", "--zap"]), &["--name"], false).unwrap();
        assert_eq!(p.value(&["--name"]).as_deref(), Some("a b"));
        assert!(p.finish(0, false).is_err());
        assert!(parse(&strs(&["--json=1"]), &[], false).is_err());
        assert!(parse(&strs(&["--name"]), &["--name"], false).is_err());
    }

    #[test]
    fn stop_at_first_command_word() {
        let mut p = parse(&strs(&["--keep", "claude", "--model", "x"]), &[], true).unwrap();
        assert!(p.flag(&["--keep"]));
        assert_eq!(p.rest, strs(&["claude", "--model", "x"]));
        assert!(p.pos.is_empty());
    }

    #[test]
    fn repeated_values_and_dash() {
        let mut p = parse(&strs(&["-l", "a=1", "-l", "b=2", "-", "-5"]), &["-l"], false).unwrap();
        assert_eq!(p.values(&["-l"]), strs(&["a=1", "b=2"]));
        assert_eq!(p.pos, strs(&["-", "-5"]));
    }

    #[test]
    fn times() {
        assert_eq!(duration_ms("250").unwrap(), 250);
        assert_eq!(duration_ms("250ms").unwrap(), 250);
        assert_eq!(duration_ms("30s").unwrap(), 30_000);
        assert_eq!(duration_ms("5m").unwrap(), 300_000);
        assert_eq!(duration_ms("1h").unwrap(), 3_600_000);
        assert!(duration_ms("soon").is_err());
        assert_eq!(number::<u16>("24", "--rows").unwrap(), 24);
        assert!(number::<u16>("-1", "--rows").is_err());
    }
}
