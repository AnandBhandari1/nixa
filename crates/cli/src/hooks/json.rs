use anyhow::{Result, anyhow, bail};

const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

pub fn parse(text: &str) -> Result<Json> {
    let src = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut i = 0;
    skip_ws(src.as_bytes(), &mut i);
    let v = value(src, &mut i, 0)?;
    skip_ws(src.as_bytes(), &mut i);
    if i != src.len() {
        bail!("extra text after the JSON at byte {i}");
    }
    Ok(v)
}

fn bad(i: usize) -> anyhow::Error {
    anyhow!("broken JSON at byte {i}")
}

fn skip_ws(s: &[u8], i: &mut usize) {
    while *i < s.len() && matches!(s[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

fn value(src: &str, i: &mut usize, depth: usize) -> Result<Json> {
    if depth > MAX_DEPTH {
        bail!("JSON is nested too deep at byte {i}");
    }
    let s = src.as_bytes();
    match s.get(*i) {
        None => bail!("JSON ends too early"),
        Some(b'{') => object(src, i, depth),
        Some(b'[') => array(src, i, depth),
        Some(b'"') => Ok(Json::Str(string(src, i)?)),
        Some(b't') => word(s, i, "true", Json::Bool(true)),
        Some(b'f') => word(s, i, "false", Json::Bool(false)),
        Some(b'n') => word(s, i, "null", Json::Null),
        Some(_) => number(src, i),
    }
}

fn word(s: &[u8], i: &mut usize, w: &str, v: Json) -> Result<Json> {
    if s[*i..].starts_with(w.as_bytes()) {
        *i += w.len();
        Ok(v)
    } else {
        Err(bad(*i))
    }
}

fn string(src: &str, i: &mut usize) -> Result<String> {
    let s = src.as_bytes();
    let start = *i;
    let mut j = start + 1;
    loop {
        match s.get(j) {
            None => return Err(bad(start)),
            Some(b'\\') => j += 2,
            Some(b'"') => break,
            Some(_) => j += 1,
        }
    }
    *i = j + 1;
    serde_json::from_str::<String>(&src[start..=j]).map_err(|_| bad(start))
}

fn number(src: &str, i: &mut usize) -> Result<Json> {
    let s = src.as_bytes();
    let start = *i;
    while *i < s.len() && matches!(s[*i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
        *i += 1;
    }
    let raw = &src[start..*i];
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Number(_)) => Ok(Json::Num(raw.to_string())),
        _ => Err(bad(start)),
    }
}

fn array(src: &str, i: &mut usize, depth: usize) -> Result<Json> {
    let s = src.as_bytes();
    *i += 1;
    let mut items = Vec::new();
    skip_ws(s, i);
    if s.get(*i) == Some(&b']') {
        *i += 1;
        return Ok(Json::Arr(items));
    }
    loop {
        skip_ws(s, i);
        items.push(value(src, i, depth + 1)?);
        skip_ws(s, i);
        match s.get(*i) {
            Some(b',') => *i += 1,
            Some(b']') => {
                *i += 1;
                return Ok(Json::Arr(items));
            }
            _ => return Err(bad(*i)),
        }
    }
}

fn object(src: &str, i: &mut usize, depth: usize) -> Result<Json> {
    let s = src.as_bytes();
    *i += 1;
    let mut pairs = Vec::new();
    skip_ws(s, i);
    if s.get(*i) == Some(&b'}') {
        *i += 1;
        return Ok(Json::Obj(pairs));
    }
    loop {
        skip_ws(s, i);
        if s.get(*i) != Some(&b'"') {
            return Err(bad(*i));
        }
        let key = string(src, i)?;
        skip_ws(s, i);
        if s.get(*i) != Some(&b':') {
            return Err(bad(*i));
        }
        *i += 1;
        skip_ws(s, i);
        let v = value(src, i, depth + 1)?;
        pairs.push((key, v));
        skip_ws(s, i);
        match s.get(*i) {
            Some(b',') => *i += 1,
            Some(b'}') => {
                *i += 1;
                return Ok(Json::Obj(pairs));
            }
            _ => return Err(bad(*i)),
        }
    }
}

pub fn pretty(v: &Json) -> String {
    let mut out = String::new();
    write(v, Some(0), &mut out);
    out.push('\n');
    out
}

pub fn compact(v: &Json) -> String {
    let mut out = String::new();
    write(v, None, &mut out);
    out
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn write(v: &Json, depth: Option<usize>, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => out.push_str(n),
        Json::Str(s) => out.push_str(&quote(s)),
        Json::Arr(items) => list(out, depth, ('[', ']'), items.len(), &|k, d, out| write(&items[k], d, out)),
        Json::Obj(pairs) => list(out, depth, ('{', '}'), pairs.len(), &|k, d, out| {
            out.push_str(&quote(&pairs[k].0));
            out.push_str(if d.is_some() { ": " } else { ":" });
            write(&pairs[k].1, d, out);
        }),
    }
}

fn list(out: &mut String, depth: Option<usize>, ends: (char, char), n: usize, item: &dyn Fn(usize, Option<usize>, &mut String)) {
    out.push(ends.0);
    if n == 0 {
        out.push(ends.1);
        return;
    }
    let inner = depth.map(|d| d + 1);
    for k in 0..n {
        if k > 0 {
            out.push(',');
        }
        if let Some(d) = inner {
            out.push('\n');
            out.push_str(&"  ".repeat(d));
        }
        item(k, inner, out);
    }
    if let Some(d) = depth {
        out.push('\n');
        out.push_str(&"  ".repeat(d));
    }
    out.push(ends.1);
}

pub fn get<'a>(v: &'a Json, key: &str) -> Option<&'a Json> {
    match v {
        Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

pub fn get_mut<'a>(v: &'a mut Json, key: &str) -> Option<&'a mut Json> {
    match v {
        Json::Obj(pairs) => pairs.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

pub fn entry<'a>(v: &'a mut Json, key: &str, default: Json) -> Option<&'a mut Json> {
    let Json::Obj(pairs) = v else { return None };
    let at = match pairs.iter().position(|(k, _)| k == key) {
        Some(at) => at,
        None => {
            pairs.push((key.to_string(), default));
            pairs.len() - 1
        }
    };
    Some(&mut pairs[at].1)
}

pub fn drop_key(v: &mut Json, key: &str) {
    if let Json::Obj(pairs) = v {
        pairs.retain(|(k, _)| k != key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_key_order_numbers_and_text() {
        let text = "{\"zeta\": 1.50, \"alpha\": [true, null, -2e3], \"s\": \"a\\\"b\\u00e9\\n\", \"e\": {}, \"l\": []}";
        let v = parse(text).unwrap();
        assert_eq!(
            compact(&v),
            "{\"zeta\":1.50,\"alpha\":[true,null,-2e3],\"s\":\"a\\\"bé\\n\",\"e\":{},\"l\":[]}"
        );
        assert_eq!(parse(&pretty(&v)).unwrap(), v);
        assert_eq!(get(&v, "s"), Some(&Json::Str("a\"bé\n".to_string())));
    }

    #[test]
    fn pretty_is_two_space_like_node() {
        let v = parse("{\"a\":[1,{\"b\":2}],\"c\":{}}").unwrap();
        assert_eq!(pretty(&v), "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"c\": {}\n}\n");
    }

    #[test]
    fn broken_text_is_refused() {
        for text in ["", "{", "{\"a\" 1}", "[1,]", "{\"a\":1} x", "\"open", "01x", "tru", "{1:2}"] {
            assert!(parse(text).is_err(), "{text:?}");
        }
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(parse(&deep).is_err());
        assert_eq!(parse("\u{feff}{}").unwrap(), Json::Obj(vec![]));
    }

    #[test]
    fn entry_and_drop() {
        let mut v = parse("{\"a\":1}").unwrap();
        *entry(&mut v, "b", Json::Null).unwrap() = Json::Bool(true);
        assert_eq!(compact(&v), "{\"a\":1,\"b\":true}");
        drop_key(&mut v, "a");
        assert_eq!(compact(&v), "{\"b\":true}");
        assert!(entry(&mut Json::Null, "x", Json::Null).is_none());
    }
}
