pub const INTERVAL_MS: u64 = 66;
const FLOOR_DB: f32 = -60.0;
const TOP_DB: f32 = -15.0;
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.15;

pub fn rms(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    ((sum / frame.len() as f64).sqrt() / 32768.0) as f32
}

pub fn loudness_of(rms: f32) -> f32 {
    if rms <= 0.0 || !rms.is_finite() {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db - FLOOR_DB) / (TOP_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

pub fn loudness(frame: &[i16]) -> f32 {
    loudness_of(rms(frame))
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

pub struct Meter {
    level: f32,
    sent_at: Option<u64>,
    sent_level: f32,
    sent_talking: bool,
    interval_ms: u64,
}

impl Default for Meter {
    fn default() -> Meter {
        Meter::new(INTERVAL_MS)
    }
}

impl Meter {
    pub fn new(interval_ms: u64) -> Meter {
        Meter { level: 0.0, sent_at: None, sent_level: -1.0, sent_talking: false, interval_ms }
    }

    fn emit(&mut self, now: u64, talking: bool, force: bool) -> Option<(f32, bool)> {
        let rounded = round2(self.level);
        if !force {
            if self.sent_at.is_some_and(|at| now.saturating_sub(at) < self.interval_ms) {
                return None;
            }
            if rounded == self.sent_level && talking == self.sent_talking {
                return None;
            }
        }
        self.sent_at = Some(now);
        self.sent_level = rounded;
        self.sent_talking = talking;
        Some((rounded, talking))
    }

    fn step(&mut self, now: u64, target: f32, talking: bool) -> Option<(f32, bool)> {
        let rate = if target > self.level { ATTACK } else { RELEASE };
        self.level += (target - self.level) * rate;
        self.emit(now, talking, false)
    }

    pub fn push(&mut self, now: u64, frame: &[i16], talking: bool) -> Option<(f32, bool)> {
        self.step(now, loudness(frame), talking)
    }

    pub fn push_rms(&mut self, now: u64, rms: f32, talking: bool) -> Option<(f32, bool)> {
        self.step(now, loudness_of(rms), talking)
    }

    pub fn reset(&mut self, now: u64) -> (f32, bool) {
        self.level = 0.0;
        self.emit(now, false, true).unwrap_or((0.0, false))
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}
