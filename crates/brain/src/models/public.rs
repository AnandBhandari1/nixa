use super::{changed, open};
use crate::app::App;
use nixa_store::models::public::{PUBLIC_MAX_BYTES, PUBLIC_SCORES_URL, apply_scores, mark_attempt, public_scores, refresh_due};
use nixa_store::models::{load, update};
use nixa_store::time::now_ms;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn fetch_scores(http: &reqwest::Client, url: &str) -> Option<Value> {
    let mut response = http.get(url).timeout(FETCH_TIMEOUT).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    if response.content_length().is_some_and(|n| n > PUBLIC_MAX_BYTES as u64) {
        return None;
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        body.extend_from_slice(&chunk);
        if body.len() > PUBLIC_MAX_BYTES {
            return None;
        }
    }
    serde_json::from_slice(&body).ok()
}

pub async fn refresh(app: &Arc<App>, force: bool) -> bool {
    refresh_from(app, PUBLIC_SCORES_URL, force).await
}

pub async fn refresh_from(app: &Arc<App>, url: &str, force: bool) -> bool {
    if !super::online::on(app) || crate::privacy::privacy_on(app) {
        return false;
    }
    let _one = app.models.refreshing.lock().await;
    if crate::privacy::privacy_on(app) {
        return false;
    }
    let Ok(conn) = open(app) else {
        return false;
    };
    let Ok(cache) = load(&conn) else {
        return false;
    };
    let now = now_ms();
    if !refresh_due(&cache, now, force) {
        return false;
    }
    if update(&conn, |cache| {
        mark_attempt(cache, now);
        Ok(())
    })
    .is_err()
    {
        return false;
    }
    drop(conn);
    let Some(raw) = fetch_scores(&app.http, url).await else {
        return false;
    };
    let scores = public_scores(&raw);
    if scores.is_empty() {
        return false;
    }
    let Ok(conn) = open(app) else {
        return false;
    };
    let done = update(&conn, |cache| apply_scores(cache, &scores, now_ms()).map(|_| ())).is_ok();
    if done {
        changed(app);
    }
    done
}
