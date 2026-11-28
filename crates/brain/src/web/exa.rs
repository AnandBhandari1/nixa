use super::Hit;
use serde_json::{Value, json};
use std::time::Duration;

pub const URL: &str = "https://api.exa.ai/search";
pub const TIMEOUT_SECS: u64 = 15;
pub const HIGHLIGHT_CHARS: usize = 600;
pub const TEXT_CHARS: usize = 700;

pub fn body(query: &str, count: usize) -> Value {
    json!({"query": query, "type": "auto", "numResults": count.clamp(1, 10), "contents": {"highlights": {"maxCharacters": HIGHLIGHT_CHARS}}})
}

pub fn short(text: &str, max: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

fn word(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn hits(data: &Value) -> Vec<Hit> {
    let Some(list) = data.get("results").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter(|r| super::web_link(&word(r, "url")))
        .map(|r| {
            let url = word(r, "url");
            let title = Some(word(r, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| url.clone());
            let highlights: Vec<&str> = r.get("highlights").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let text = if highlights.is_empty() { word(r, "text") } else { highlights.join(" … ") };
            Hit { title, url, date: word(r, "publishedDate").chars().take(10).collect(), text: short(&text, TEXT_CHARS) }
        })
        .collect()
}

pub fn status_error(code: u16) -> String {
    match code {
        401 | 403 => "Exa refused the key.".into(),
        429 => "Exa says too many searches, or there is no credit left (429).".into(),
        _ => format!("Exa search failed ({code})."),
    }
}

pub async fn search(http: &reqwest::Client, url: &str, key: &str, query: &str, count: usize) -> Result<Vec<Hit>, String> {
    let res = http
        .post(url)
        .header("x-api-key", key)
        .json(&body(query, count))
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "Exa did not answer in time.".to_string() } else { "Exa could not be reached.".to_string() })?;
    if !res.status().is_success() {
        return Err(status_error(res.status().as_u16()));
    }
    let data: Value = res.json().await.map_err(|_| "Exa sent an answer that is not JSON.".to_string())?;
    Ok(hits(&data))
}
