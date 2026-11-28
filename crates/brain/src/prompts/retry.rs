use super::link::Link;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFrom {
    Screen,
    Status,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenRetry {
    pub text: Option<String>,
    pub tries: u32,
    pub from: ReadFrom,
    pub status: Option<String>,
    pub problem: Option<String>,
}

pub const TRIES: u32 = 2;

pub async fn read_with_retry(link: &Link, terminal: &str) -> ScreenRetry {
    let mut problem = String::new();
    for tries in 1..=TRIES {
        match link.read(terminal).await {
            Ok(text) if !text.trim().is_empty() => return ScreenRetry { text: Some(text), tries, from: ReadFrom::Screen, status: None, problem: None },
            Ok(_) => problem = "read returned no text".into(),
            Err(e) => problem = e.to_string(),
        }
    }
    let line = link.get(terminal).await.ok().and_then(|t| t.status_note).map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let first = if problem.is_empty() { "read failed or was blocked".to_string() } else { problem };
    match line {
        Some(line) => {
            let short: String = line.chars().take(120).collect();
            let problem = format!("{first} twice; the terminal status says \"{short}\", so it is not read as a finished screen");
            ScreenRetry { text: None, tries: TRIES, from: ReadFrom::Status, status: Some(line), problem: Some(problem) }
        }
        None => ScreenRetry { text: None, tries: TRIES, from: ReadFrom::None, status: None, problem: Some(format!("{first} twice, and the terminal has no status line")) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_engine_gives_a_clear_problem() {
        let link = Link::new("/tmp/mo-none/e.sock");
        let got = read_with_retry(&link, "m1").await;
        assert_eq!(got.from, ReadFrom::None);
        assert_eq!(got.tries, 2);
        assert!(got.text.is_none());
        assert!(got.problem.unwrap().contains("twice, and the terminal has no status line"));
    }
}
