use crate::door::{Door, Line, Out, State};
use crate::wire::{self, PlayedHow};
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::oneshot;

impl State {
    pub(crate) fn voice_conns(&self) -> Vec<u64> {
        self.conns.iter().filter(|(_, c)| c.voice && !c.text_only && c.open()).map(|(no, _)| *no).collect()
    }

    pub(crate) fn finish_line(&mut self, line: u32, how: PlayedHow) {
        let Some(rec) = self.lines.get_mut(&line) else {
            return;
        };
        match rec.waiter.take() {
            Some(waiter) => {
                self.lines.remove(&line);
                let _ = waiter.send(how);
            }
            None => rec.done = Some(how),
        }
    }

    pub(crate) fn drop_from_lines(&mut self, no: u64) {
        let ended: Vec<(u32, bool)> = self
            .lines
            .iter_mut()
            .filter_map(|(line, rec)| (rec.targets.remove(&no) && rec.targets.is_empty()).then_some((*line, rec.any_played)))
            .collect();
        for (line, any_played) in ended {
            self.finish_line(line, if any_played { PlayedHow::Played } else { PlayedHow::Gone });
        }
    }
}

impl Door {
    pub fn audio_ready(&self) -> usize {
        self.lock().voice_conns().len()
    }

    pub fn audio_start(&self, line: u32, rate: u32) -> bool {
        let mut st = self.lock();
        let targets = st.voice_conns();
        if targets.is_empty() {
            return false;
        }
        let text = wire::audio_start(line, rate);
        for no in &targets {
            if let Some(conn) = st.conns.get(no) {
                conn.send(Out::Text(text.clone()));
            }
        }
        let targets: HashSet<u64> = targets.into_iter().collect();
        st.lines.insert(line, Line { targets, any_played: false, done: None, waiter: None });
        true
    }

    pub fn audio_chunk(&self, line: u32, part: u32, pcm: &[u8]) {
        let st = self.lock();
        let Some(rec) = st.lines.get(&line) else {
            return;
        };
        let frame = wire::audio_frame(line, part, pcm);
        for no in &rec.targets {
            if let Some(conn) = st.conns.get(no).filter(|c| c.open()) {
                conn.send(Out::Binary(frame.clone()));
            }
        }
    }

    pub fn audio_end(&self, line: u32, parts: u32) {
        let st = self.lock();
        let Some(rec) = st.lines.get(&line) else {
            return;
        };
        let text = wire::audio_end(line, parts);
        for no in &rec.targets {
            if let Some(conn) = st.conns.get(no) {
                conn.send(Out::Text(text.clone()));
            }
        }
    }

    pub fn audio_stop(&self, line: u32) {
        let mut st = self.lock();
        let text = wire::audio_stop(line);
        for no in st.voice_conns() {
            if let Some(conn) = st.conns.get(&no) {
                conn.send(Out::Text(text.clone()));
            }
        }
        st.finish_line(line, PlayedHow::Stopped);
        st.lines.remove(&line);
    }

    pub async fn audio_played(&self, line: u32, timeout_ms: u64) -> PlayedHow {
        let rx = {
            let mut st = self.lock();
            let Some(rec) = st.lines.get_mut(&line) else {
                return PlayedHow::Gone;
            };
            if let Some(done) = rec.done {
                st.lines.remove(&line);
                return done;
            }
            let (tx, rx) = oneshot::channel();
            rec.waiter = Some(tx);
            rx
        };
        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(how)) => how,
            Ok(Err(_)) => PlayedHow::Gone,
            Err(_) => {
                self.lock().lines.remove(&line);
                PlayedHow::Timeout
            }
        }
    }
}
