use gpui::{App, Global, Hsla, Rgba, rgb};
use nixa_protocol::{Cell, Color, flags};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub selection: Hsla,
    pub cursor: Hsla,
    pub ansi: [Hsla; 16],
}

fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

pub fn one_dark() -> Palette {
    Palette {
        background: hex(0x282c34),
        foreground: hex(0xabb2bf),
        selection: hex(0x334659),
        cursor: hex(0x61afef),
        ansi: [
            hex(0x3e4451),
            hex(0xe06c75),
            hex(0x98c379),
            hex(0xe5c07b),
            hex(0x61afef),
            hex(0xc678dd),
            hex(0x56b6c2),
            hex(0xabb2bf),
            hex(0x5c6370),
            hex(0xe06c75),
            hex(0x98c379),
            hex(0xe5c07b),
            hex(0x61afef),
            hex(0xc678dd),
            hex(0x56b6c2),
            hex(0xffffff),
        ],
    }
}

impl Default for Palette {
    fn default() -> Palette {
        one_dark()
    }
}

impl Global for Palette {}

pub fn current(cx: &App) -> Palette {
    cx.try_global::<Palette>().copied().unwrap_or_default()
}

pub fn set_current(cx: &mut App, palette: Palette) {
    if cx.try_global::<Palette>() != Some(&palette) {
        cx.set_global(palette);
        cx.refresh_windows();
    }
}

fn rgb_hsla(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

pub fn indexed(p: &Palette, i: u8) -> Hsla {
    match i {
        0..=15 => p.ansi[i as usize],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            rgb_hsla(level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            rgb_hsla(v, v, v)
        }
    }
}

pub fn resolve(p: &Palette, c: Color) -> Option<Hsla> {
    match c {
        Color::Default => None,
        Color::Indexed(i) => Some(indexed(p, i)),
        Color::Rgb(r, g, b) => Some(rgb_hsla(r, g, b)),
    }
}

pub fn cell_foreground(p: &Palette, cell: &Cell) -> Hsla {
    if cell.has(flags::HIDDEN) {
        return cell_background(p, cell).unwrap_or(p.background);
    }
    let fg = if cell.has(flags::INVERSE) {
        resolve(p, cell.bg).unwrap_or(p.background)
    } else {
        resolve(p, cell.fg).unwrap_or(p.foreground)
    };
    if cell.has(flags::DIM) {
        fg.opacity(0.6)
    } else {
        fg
    }
}

pub fn cell_background(p: &Palette, cell: &Cell) -> Option<Hsla> {
    if cell.has(flags::INVERSE) {
        Some(resolve(p, cell.fg).unwrap_or(p.foreground))
    } else {
        resolve(p, cell.bg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grey_ranges() {
        let p = one_dark();
        assert_eq!(indexed(&p, 1), p.ansi[1]);
        assert_eq!(indexed(&p, 16), rgb_hsla(0, 0, 0));
        assert_eq!(indexed(&p, 231), rgb_hsla(255, 255, 255));
        assert_eq!(indexed(&p, 232), rgb_hsla(8, 8, 8));
        assert_eq!(indexed(&p, 255), rgb_hsla(238, 238, 238));
    }

    #[test]
    fn inverse_swaps_colors() {
        let p = one_dark();
        let cell = Cell {
            flags: flags::INVERSE,
            ..Cell::default()
        };
        assert_eq!(cell_foreground(&p, &cell), p.background);
        assert_eq!(cell_background(&p, &cell), Some(p.foreground));
    }

    #[test]
    fn hidden_text_uses_the_effective_background_without_dimming() {
        let p = one_dark();
        let mut cell = Cell {
            fg: Color::Indexed(1),
            bg: Color::Indexed(2),
            flags: flags::HIDDEN | flags::DIM,
            ..Cell::default()
        };
        assert_eq!(cell_foreground(&p, &cell), p.ansi[2]);
        cell.flags |= flags::INVERSE;
        assert_eq!(cell_foreground(&p, &cell), p.ansi[1]);
        assert_eq!(cell_background(&p, &cell), Some(p.ansi[1]));
    }
}
