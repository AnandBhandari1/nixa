pub const PAUSE_MS: u64 = 3000;
pub const SILENCE_MS: u64 = 700;
pub const GATHER_MAX_MS: u64 = 180_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub started_at: u64,
    pub captured_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub parts: Vec<String>,
    pub started_at: u64,
    pub captured_at: u64,
}

impl Message {
    pub fn text(&self) -> String {
        self.parts.join(" ")
    }
}

pub fn pause_wait(pause_ms: u64, silence_ms: u64) -> u64 {
    pause_ms.saturating_sub(silence_ms)
}

pub struct Gather {
    wait_ms: u64,
    max_ms: u64,
    pieces: Vec<Piece>,
    pause_at: Option<u64>,
    cap_at: Option<u64>,
    talking: bool,
}

impl Default for Gather {
    fn default() -> Gather {
        Gather::new(pause_wait(PAUSE_MS, SILENCE_MS), GATHER_MAX_MS)
    }
}

impl Gather {
    pub fn new(wait_ms: u64, max_ms: u64) -> Gather {
        Gather { wait_ms, max_ms, pieces: Vec::new(), pause_at: None, cap_at: None, talking: false }
    }

    fn arm(&mut self, now: u64) {
        self.pause_at = (!self.pieces.is_empty()).then_some(now + self.wait_ms);
    }

    fn arm_cap(&mut self, now: u64) {
        if self.cap_at.is_none() && !self.pieces.is_empty() {
            self.cap_at = Some(now + self.max_ms);
        }
    }

    pub fn speech_start(&mut self) {
        self.talking = true;
        self.pause_at = None;
    }

    pub fn speech_end(&mut self, now: u64) {
        self.talking = false;
        self.arm(now);
    }

    pub fn phrase(&mut self, now: u64, piece: Piece) -> Option<Message> {
        self.pieces.push(piece);
        if now.saturating_sub(self.pieces[0].started_at) >= self.max_ms {
            return self.flush();
        }
        self.arm_cap(now);
        if self.talking {
            self.pause_at = None;
        } else {
            self.arm(now);
        }
        None
    }

    pub fn due(&mut self, now: u64) -> Option<Message> {
        if self.cap_at.is_some_and(|at| now >= at) {
            return self.flush();
        }
        if self.pause_at.is_some_and(|at| now >= at) {
            self.pause_at = None;
            if !self.talking {
                return self.flush();
            }
        }
        None
    }

    pub fn flush(&mut self) -> Option<Message> {
        self.pause_at = None;
        self.cap_at = None;
        if self.pieces.is_empty() {
            return None;
        }
        let batch = std::mem::take(&mut self.pieces);
        let parts: Vec<String> = batch.iter().map(|p| p.text.trim().to_string()).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return None;
        }
        Some(Message { parts, started_at: batch[0].started_at, captured_at: batch[batch.len() - 1].captured_at })
    }

    pub fn drop_all(&mut self) {
        self.pause_at = None;
        self.cap_at = None;
        self.pieces.clear();
        self.talking = false;
    }

    pub fn pending(&self) -> bool {
        !self.pieces.is_empty()
    }

    pub fn talking(&self) -> bool {
        self.talking
    }
}
