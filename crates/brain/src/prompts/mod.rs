pub mod answer;
pub mod ask_match;
pub mod auto;
pub mod book;
pub mod card;
pub mod deliver;
pub mod decide_first;
pub mod dialog;
pub mod instructions;
pub mod link;
pub mod notes;
pub mod retry;
pub mod routes;
pub mod rules_block;
pub mod scan;
pub mod screen;
pub mod tools;

#[cfg(test)]
mod live_tests;

use crate::app::{App, lock};
use book::{Book, Prompt};
use link::Link;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use routes::{routes, ws};
pub use tools::{call, tools};

pub const TICK_MS: u64 = 1500;

#[derive(Default)]
pub struct State {
    pub book: Mutex<Book>,
    pub socket: Mutex<Option<PathBuf>>,
}

impl State {
    pub fn link(&self) -> Link {
        match lock(&self.socket).clone() {
            Some(socket) => Link::new(socket),
            None => Link::default(),
        }
    }

    pub fn set_socket(&self, socket: Option<PathBuf>) {
        *lock(&self.socket) = socket;
    }
}

pub fn prompt_text(prompt: &Prompt) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(q) = prompt.question() {
        parts.push(q.question.clone());
        parts.extend(q.options.iter().map(|o| o.label.clone()));
    }
    parts.extend([prompt.text.clone(), prompt.detail.clone(), prompt.tool_name.clone()].into_iter().flatten());
    parts.join(" ")
}

pub fn snapshot(app: &App) -> Value {
    let (first, sending, waiting, last_sent) = {
        let book = lock(&app.prompts.book);
        (book.items.first().cloned(), book.sending.clone(), book.items.len().saturating_sub(1), book.last_sent.clone())
    };
    let card = first.map(|p| card::card_of(&p, sending.as_deref(), true));
    json!({
        "type": "prompts",
        "card": card,
        "waiting": waiting,
        "last_sent": last_sent,
    })
}

pub fn broadcast(app: &App) {
    app.send_out(snapshot(app));
}

pub async fn tick(app: &Arc<App>) {
    answer::tick(app).await;
}

pub async fn run(app: Arc<App>) {
    let mut timer = tokio::time::interval(Duration::from_millis(TICK_MS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        tick(&app).await;
    }
}
