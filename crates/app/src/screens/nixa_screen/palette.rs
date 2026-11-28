use super::mode::{Mode, Tone};
use gpui::{Hsla, hsla, rgb};
use nixa_ui::Theme;

const PINK_HUE: f32 = 330.0;
const ROSE_HUE: f32 = 345.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tones {
    pub main: Hsla,
    pub second: Hsla,
    pub third: Hsla,
}

#[derive(Clone, Copy)]
struct Base {
    sat: f32,
    light: f32,
}

fn ansi(t: &Theme, i: usize) -> Hsla {
    t.terminal.ansi[i]
}

fn base_of(t: &Theme) -> Base {
    let base = ansi(t, 13);
    Base {
        sat: base.s.clamp(0.55, 0.9),
        light: base.l.clamp(0.62, 0.74),
    }
}

fn hue(base: Base, deg: f32, lift: f32) -> Hsla {
    hsla(
        deg.rem_euclid(360.0) / 360.0,
        base.sat.clamp(0.0, 1.0),
        (base.light + lift).clamp(0.0, 0.95),
        1.0,
    )
}

pub fn pink(t: &Theme) -> (Hsla, Hsla) {
    let base = base_of(t);
    (hue(base, PINK_HUE, 0.0), hue(base, ROSE_HUE, 0.08))
}

fn set(a: Hsla, b: Hsla, c: Hsla) -> Tones {
    Tones {
        main: a,
        second: b,
        third: c,
    }
}

fn hues(t: &Theme, [a, b, c]: [f32; 3]) -> Tones {
    let base = base_of(t);
    set(hue(base, a, 0.0), hue(base, b, -0.04), hue(base, c, 0.1))
}

pub fn palette_of(tone: Tone, t: &Theme) -> Tones {
    let (pink, rose) = pink(t);
    let standby = set(ansi(t, 4), ansi(t, 5), ansi(t, 6));
    match tone {
        Tone::Deep => hues(t, [46.0, 38.0, 56.0]),
        Tone::Agent => hues(t, [24.0, 12.0, 30.0]),
        Tone::Mode(mode) => match mode {
            Mode::Off => set(t.dim, t.muted, t.dim),
            Mode::Wait => set(ansi(t, 4), ansi(t, 6), ansi(t, 5)),
            Mode::You => set(ansi(t, 2), ansi(t, 6), ansi(t, 3)),
            Mode::Nixa => set(pink, ansi(t, 5), rose),
            Mode::Think => set(ansi(t, 5), ansi(t, 4), pink),
            Mode::Paused => set(
                rgb(0xc9d1dc).into(),
                rgb(0x8b95a3).into(),
                rgb(0xe8edf3).into(),
            ),
            Mode::Listen => set(ansi(t, 4), ansi(t, 6), ansi(t, 2)),
            Mode::Standby | Mode::MicOff => standby,
        },
    }
}

pub fn status_color(t: &Theme, status: super::map::St) -> Hsla {
    use super::map::St;
    match status {
        St::Blocked => ansi(t, 3),
        St::Working => ansi(t, 6),
        St::Done => ansi(t, 2),
        St::Idle => rgb(0x5b6472).into(),
    }
}

pub fn glow_color(t: &Theme, status: super::map::St) -> Option<Hsla> {
    use super::map::St;
    match status {
        St::Blocked => Some(ansi(t, 1)),
        St::Working => Some(ansi(t, 6)),
        St::Done => Some(ansi(t, 2)),
        St::Idle => None,
    }
}

pub fn mix(a: Hsla, b: Hsla, k: f32) -> Hsla {
    let a = a.to_rgb();
    let b = b.to_rgb();
    gpui::Rgba {
        r: a.r + (b.r - a.r) * k,
        g: a.g + (b.g - a.g) * k,
        b: a.b + (b.b - a.b) * k,
        a: 1.0,
    }
    .into()
}

pub fn aura_color(id: &str) -> Hsla {
    let mut seed: u32 = 2_166_136_261;
    for ch in id.chars() {
        seed = (seed ^ ch as u32).wrapping_mul(16_777_619);
    }
    loop {
        let value = seed % (160 * 160 * 160);
        let rgb3 = [
            64 + value % 160,
            64 + (value / 160) % 160,
            64 + value / (160 * 160),
        ];
        let max = rgb3.iter().copied().max().unwrap_or(0);
        let min = rgb3.iter().copied().min().unwrap_or(0);
        seed = seed.wrapping_add(0x9e3779);
        if max >= 160 && max - min >= 40 {
            return rgb((rgb3[0] << 16) | (rgb3[1] << 8) | rgb3[2]).into();
        }
    }
}
