use crate::screens::chat::model::{Link, Snapshot};

pub const TALK_HOLD_MS: u128 = 700;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Wait,
    You,
    Nixa,
    Think,
    Paused,
    Listen,
    Standby,
    MicOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Mode(Mode),
    Deep,
    Agent,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Off => "OFFLINE",
            Mode::Wait => "CONNECTING",
            Mode::You => "HEARING YOU",
            Mode::Nixa => "SPEAKING",
            Mode::Think => "THINKING",
            Mode::Paused => "PAUSED",
            Mode::Listen => "LISTENING",
            Mode::Standby => "STANDBY",
            Mode::MicOff => "MIC OFF",
        }
    }

    pub fn animated(self) -> bool {
        matches!(self, Mode::You | Mode::Nixa | Mode::Think)
    }

    pub fn hearing(self) -> bool {
        matches!(self, Mode::You | Mode::Listen | Mode::Standby)
    }

    pub fn idle(self) -> bool {
        matches!(
            self,
            Mode::Off | Mode::Wait | Mode::Standby | Mode::Paused | Mode::MicOff | Mode::Listen
        )
    }

    pub fn glow(self) -> f32 {
        match self {
            Mode::Off => 0.25,
            Mode::Wait => 0.5,
            Mode::You => 0.8,
            Mode::Nixa => 0.9,
            Mode::Think => 0.75,
            Mode::Paused => 0.4,
            Mode::Listen => 0.65,
            Mode::Standby => 0.55,
            Mode::MicOff => 0.4,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Feed {
    pub up: bool,
    pub off: bool,
    pub name: String,
    pub state: String,
    pub busy: bool,
    pub mic_off: bool,
    pub hearing: bool,
}

impl Feed {
    pub fn name(&self) -> &str {
        if self.name.trim().is_empty() {
            "Nixa"
        } else {
            self.name.trim()
        }
    }
}

pub fn feed_of(snap: &Snapshot, off: bool, now_ms: u64) -> Feed {
    Feed {
        up: snap.link == Link::Up,
        off,
        name: snap.name().to_string(),
        state: snap.state.clone(),
        busy: snap.busy,
        mic_off: snap.mic_off,
        hearing: snap.hearing || snap.wake_active || snap.wake_until > now_ms,
    }
}

pub fn mode_of(feed: &Feed, talking: bool) -> Mode {
    if !feed.up {
        return if feed.off { Mode::Off } else { Mode::Wait };
    }
    if talking {
        return Mode::You;
    }
    match feed.state.as_str() {
        "speaking" => return Mode::Nixa,
        "thinking" => return Mode::Think,
        "paused" => return Mode::Paused,
        "off" => return Mode::Off,
        _ => {}
    }
    if feed.busy {
        return Mode::Think;
    }
    if feed.mic_off {
        return Mode::MicOff;
    }
    if feed.hearing {
        Mode::Listen
    } else {
        Mode::Standby
    }
}

pub fn shown_of(real: Mode, deep_working: bool) -> (Mode, Tone) {
    if real == Mode::Nixa {
        return (Mode::Nixa, Tone::Mode(Mode::Nixa));
    }
    if deep_working && matches!(real, Mode::Think | Mode::Listen | Mode::Standby) {
        return (Mode::Think, Tone::Deep);
    }
    (real, Tone::Mode(real))
}

pub fn energy_of(mode: Mode, t: f32, level: f32) -> f32 {
    match mode {
        Mode::You => level.clamp(0.0, 1.0),
        Mode::Nixa => {
            (0.45 + 0.25 * (t * 8.7).sin() + 0.18 * (t * 21.3 + 1.1).sin() + 0.1 * (t * 3.1).sin())
                .clamp(0.0, 1.0)
        }
        Mode::Think => 0.25 + 0.1 * (t * 3.0).sin(),
        _ => 0.0,
    }
}

pub const START_GLOW: f32 = 0.5;

pub fn next_glow(glow: f32, mode: Mode, dt: f32) -> f32 {
    glow + (mode.glow() - glow) * (dt * 3.0).clamp(0.0, 1.0)
}

pub fn ease(value: f32, goal: f32) -> f32 {
    let k = if goal > value { 0.5 } else { 0.18 };
    let next = value + (goal - value) * k;
    if next < 0.005 { 0.0 } else { next }
}
