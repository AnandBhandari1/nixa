#[path = "read_window.rs"]
pub mod read_window;

use super::speech::SayOptions;
use crate::app::{App, lock};
use crate::util::{log, now_ms};
use nixa_voice::{SayResult, scrub_for_speech};
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub const RECORD_MAX_MS: u64 = nixa_phone::wire::TALK_MAX_MS as u64;
pub const ANSWER_WAIT_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Notice,
    Answer,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadMark {
    pub task: i64,
    pub read: u64,
}

pub struct Line {
    pub text: String,
    pub options: SayOptions,
    key: String,
    read: Option<ReadMark>,
    waiters: Vec<oneshot::Sender<SayResult>>,
}

#[derive(Default)]
pub struct Hold {
    phones: BTreeSet<String>,
    since: u64,
    answer_until: u64,
    lines: VecDeque<Line>,
    draining: bool,
    reads: read_window::Reads,
}

pub fn key(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect::<Vec<_>>().join(" ")
}

impl Hold {
    pub fn recording(&self) -> bool {
        !self.phones.is_empty()
    }

    pub fn on(&self) -> bool {
        self.recording() || self.answer_until > 0
    }

    pub fn holds(&self, pass: Pass) -> bool {
        self.recording() || (self.answer_until > 0 && pass == Pass::Notice)
    }

    pub fn held(&self) -> usize {
        self.lines.len()
    }

    pub fn texts(&self) -> Vec<String> {
        self.lines.iter().map(|l| l.text.clone()).collect()
    }

    pub fn start(&mut self, phone: &str, now: u64) {
        if self.phones.is_empty() {
            self.since = now;
        }
        self.phones.insert(phone.to_string());
        self.answer_until = 0;
    }

    pub fn end(&mut self, phone: &str, now: u64) -> bool {
        if !self.phones.remove(phone) {
            return false;
        }
        if self.phones.is_empty() {
            self.answer_until = now + ANSWER_WAIT_MS;
        }
        true
    }

    pub fn cancel(&mut self, phone: &str) -> bool {
        if !self.phones.remove(phone) {
            return false;
        }
        if self.phones.is_empty() {
            self.answer_until = 0;
        }
        true
    }

    pub fn answered(&mut self) -> bool {
        if self.recording() || self.answer_until == 0 {
            return false;
        }
        self.answer_until = 0;
        true
    }

    pub fn due(&self, now: u64) -> bool {
        if self.recording() {
            return now.saturating_sub(self.since) >= RECORD_MAX_MS;
        }
        self.answer_until > 0 && now >= self.answer_until
    }

    pub fn expire(&mut self) {
        self.phones.clear();
        self.answer_until = 0;
    }

    pub fn keep(&mut self, text: &str, options: &SayOptions) -> oneshot::Receiver<SayResult> {
        let (tx, rx) = oneshot::channel();
        let key = key(text);
        match self.lines.iter_mut().find(|l| l.key == key && l.read.is_none()) {
            Some(line) => line.waiters.push(tx),
            None => self.lines.push_back(Line { text: text.to_string(), options: options.clone(), key, read: None, waiters: vec![tx] }),
        }
        rx
    }

    fn keep_read(&mut self, text: &str, options: &SayOptions, mark: ReadMark) {
        self.lines.push_back(Line { text: text.to_string(), options: options.clone(), key: key(text), read: Some(mark), waiters: Vec::new() });
    }

    #[cfg(test)]
    pub fn reads(&self) -> Vec<ReadMark> {
        self.lines.iter().filter_map(|l| l.read).collect()
    }

    pub fn begin_drain(&mut self) -> bool {
        if self.on() || self.draining || self.lines.is_empty() {
            return false;
        }
        self.draining = true;
        true
    }

    pub fn clear(&mut self) -> Vec<Line> {
        self.answer_until = 0;
        self.lines.drain(..).collect()
    }

    pub fn put_back(&mut self, line: Line) {
        self.lines.push_front(line);
        self.draining = false;
    }

    pub fn give_back(&mut self, line: Line) -> bool {
        self.put_back(line);
        self.begin_drain()
    }

    pub fn take_next(&mut self) -> Option<Line> {
        let line = if self.on() { None } else { self.lines.pop_front() };
        if line.is_none() {
            self.draining = false;
        }
        line
    }
}

impl Line {
    pub fn done(self, result: SayResult) {
        for waiter in self.waiters {
            let _ = waiter.send(result);
        }
    }
}

pub fn holding(app: &App) -> bool {
    lock(&app.talk.hold).on()
}

pub fn catch(app: &App, text: &str, options: &SayOptions) -> bool {
    let clean = scrub_for_speech(text).trim().to_string();
    if clean.chars().count() < 2 {
        return false;
    }
    let mut hold = lock(&app.talk.hold);
    if !hold.holds(Pass::Notice) {
        return false;
    }
    drop(hold.keep(&clean, options));
    log("hold", &format!("held a spoken line, {} waiting", hold.held()));
    true
}

pub fn start(app: &App, phone: &str) {
    lock(&app.talk.hold).start(phone, now_ms());
    log("hold", &format!("{phone} records, spoken lines wait"));
}

pub fn end(app: &Arc<App>, phone: &str) {
    if lock(&app.talk.hold).end(phone, now_ms()) {
        log("hold", &format!("{phone} stopped recording, the answer goes first"));
    }
}

pub fn cancel(app: &Arc<App>, phone: &str) {
    if lock(&app.talk.hold).cancel(phone) {
        log("hold", &format!("{phone} cancelled or left"));
    }
    release(app);
}

pub fn answered(app: &Arc<App>) {
    if lock(&app.talk.hold).answered() {
        log("hold", "the answer is out");
    }
    release(app);
}

pub fn silence(app: &App) {
    let lines = {
        let mut hold = lock(&app.talk.hold);
        let lines = hold.clear();
        for mark in lines.iter().filter_map(|line| line.read) {
            hold.reads.finish(mark.read);
        }
        lines
    };
    if !lines.is_empty() {
        log("hold", &format!("dropped {} held lines, the voice is off", lines.len()));
    }
    for line in lines {
        line.done(SayResult { interrupted: true, ms: 0 });
    }
}

pub fn read_task(app: &Arc<App>, text: &str, task: i64) -> Option<ReadMark> {
    let (mark, held) = begin_read(app, text, task)?;
    held.then_some(mark)
}

fn begin_read(app: &Arc<App>, text: &str, task: i64) -> Option<(ReadMark, bool)> {
    let clean = scrub_for_speech(text).trim().to_string();
    if clean.chars().count() < 2 {
        return None;
    }
    let options = SayOptions::nixa("chat");
    let (mark, stop) = {
        let mut hold = lock(&app.talk.hold);
        let (read, stop) = hold.reads.start();
        let mark = ReadMark { task, read };
        if hold.holds(Pass::Notice) {
            hold.keep_read(&clean, &options, mark);
            log("hold", &format!("held the read of task {task}, {} waiting", hold.held()));
            return Some((mark, true));
        }
        (mark, stop)
    };
    let app = app.clone();
    tokio::spawn(async move {
        while super::speech::say_clean(&app, &clean, options.clone(), Pass::Notice, Some(&stop)).await.is_none() {
            let mut hold = lock(&app.talk.hold);
            if stop.is_cancelled() {
                break;
            }
            if hold.holds(Pass::Notice) {
                hold.keep_read(&clean, &options, mark);
                return;
            }
        }
        lock(&app.talk.hold).reads.finish(mark.read);
    });
    Some((mark, false))
}

pub fn stop_reads(app: &App) -> usize {
    stop_read(app, None)
}

fn stop_read(app: &App, read: Option<u64>) -> usize {
    let count = {
        let mut hold = lock(&app.talk.hold);
        hold.reads.stop(read);
        let (mut lines, mut count) = (std::mem::take(&mut hold.lines), 0);
        lines.retain(|line| {
            if let Some(mark) = line.read.filter(|mark| read.is_none_or(|id| id == mark.read)) {
                hold.reads.finish(mark.read);
                count += 1;
                false
            } else {
                true
            }
        });
        hold.lines = lines;
        count
    };
    if count > 0 {
        log("hold", &format!("dropped {count} held reads, the user stopped them"));
    }
    count
}

pub fn tick(app: &Arc<App>, now: u64) {
    let due = {
        let mut hold = lock(&app.talk.hold);
        let due = hold.due(now);
        if due {
            hold.expire();
        }
        due
    };
    if due {
        log("hold", "the hold ran out of time");
        release(app);
    }
}

pub fn release(app: &Arc<App>) {
    if !lock(&app.talk.hold).begin_drain() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move { drain(&app).await });
}

async fn drain(app: &Arc<App>) {
    loop {
        let (line, stop) = {
            let mut hold = lock(&app.talk.hold);
            let Some(line) = hold.take_next() else { break };
            let stop = line.read.and_then(|mark| hold.reads.stop_token(mark.read));
            (line, stop)
        };
        match super::speech::say_clean(app, &line.text, line.options.clone(), Pass::Notice, stop.as_ref()).await {
            Some(result) => {
                if let Some(mark) = line.read {
                    lock(&app.talk.hold).reads.finish(mark.read);
                }
                line.done(result);
            }
            None => {
                let mut hold = lock(&app.talk.hold);
                if let Some(mark) = line.read
                    && stop.as_ref().is_some_and(CancellationToken::is_cancelled)
                {
                    hold.reads.finish(mark.read);
                    continue;
                }
                if !hold.give_back(line) {
                    break;
                }
            }
        }
    }
}
