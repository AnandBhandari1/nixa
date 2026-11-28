use super::brush::{Brush, hex};
use super::motion;
use super::plan::Role;
use crate::screens::nixa_screen::map::deep_color;
use crate::screens::nixa_screen::palette::{aura_color, mix};
use gpui::Hsla;

pub const W: f32 = 14.0;
pub const H: f32 = 20.0;
pub const SIT_ROWS: usize = 14;
pub const SIT_TOP: f32 = 14.0;
pub const SOFA_TOP: f32 = 17.0;

const SKIN: [u32; 6] = [0xf1c27d, 0xe0ac69, 0xc68642, 0x8d5524, 0xffdbac, 0x6b3f22];
const HAIR: [u32; 9] = [
    0x2b1d14, 0x4a2f1b, 0x111114, 0x8a5a2b, 0xd9b26a, 0x6b6b6b, 0xa8432f, 0x3d2a5c, 0x1f4a6b,
];
const PANTS: [u32; 4] = [0x2d3142, 0x3a3f58, 0x1f2a44, 0x4b4f5c];
const LINE: u32 = 0x1d1a24;
const EYE: u32 = 0x1b1b1f;
const COLLAR: u32 = 0xf4f1ea;
const BELT: u32 = 0x232028;
const TIE: u32 = 0x2a2440;
const LENS: u32 = 0xcfe8ff;
const MOUTH_OPEN: u32 = 0x7a3b2e;
const SHADOW: u32 = 0x2a1e14;

type Grid = [&'static [u8; 14]];

const FRONT: [&[u8; 14]; 20] = [
    b"....oooooo....",
    b"...ohhHHhho...",
    b"..ohhhhhhhho..",
    b"..ohhhsshhho..",
    b"..ohsssssSho..",
    b"..ossessesso..",
    b"..osssmmssso..",
    b"...oSssssSo...",
    b"....oSSSSo....",
    b"..oTtcttctTo..",
    b".oTtttttttttTo",
    b".oTtttttttttTo",
    b".osTtttttttTso",
    b"..oTttttttTo..",
    b"..okkkkkkkko..",
    b"..opppPPpppo..",
    b"..opppoopppo..",
    b"..oppo..oppo..",
    b".obbbo..obbbo.",
    b"..ooo....ooo..",
];

const BACK: [&[u8; 14]; 20] = [
    b"....oooooo....",
    b"...ohhHHhho...",
    b"..ohhhhhhhho..",
    b"..ohhhhhhhho..",
    b"..ohhhhhhhho..",
    b"..ohhhhhhhho..",
    b"..ohhhhhhhho..",
    b"...ohhhhhho...",
    b"....oSSSSo....",
    b"..oTttttttTo..",
    b".oTtttttttttTo",
    b".oTtttttttttTo",
    b".osTtttttttTso",
    b"..oTttttttTo..",
    b"..okkkkkkkko..",
    b"..opppPPpppo..",
    b"..opppoopppo..",
    b"..oppo..oppo..",
    b".obbbo..obbbo.",
    b"..ooo....ooo..",
];

const SIDE: [&[u8; 14]; 20] = [
    b"....oooooo....",
    b"...ohhHHhho...",
    b"..ohhhhhhhho..",
    b"..ohhhhsssso..",
    b"..ohhhhssSso..",
    b"..ohhhhsseso..",
    b"..ohhhhssmsSo.",
    b"...ohhSsssSo..",
    b"....oSSSSo....",
    b"...otttttttto.",
    b"...otttTtttto.",
    b"...otttTtttto.",
    b"...otttstttto.",
    b"...otttttttto.",
    b"...okkkkkkkko.",
    b"...oppppppppo.",
    b"....oppppppo..",
    b".....oppppo...",
    b".....obbbbbo..",
    b"......ooooo...",
];

const SIDE_SCISSOR: [&[u8; 14]; 4] = [
    b"...opppppppo..",
    b"..oppo...oppo.",
    b".obbo....obbbo",
    b"..oo......ooo.",
];

const WALK_A: [&[u8; 14]; 4] = [
    b"..opppoopppo..",
    b".oppo....oppo.",
    b".obbo....obbo.",
    b"..oo......oo..",
];

const WALK_B: [&[u8; 14]; 4] = [
    b"..opppoopppo..",
    b"...oppoppo....",
    b"...obbobbo....",
    b"....oo.oo.....",
];

const SOFA_LEGS: [&[u8; 14]; 3] = [
    b"..oppppppppo..",
    b"..oPpppppPpo..",
    b"..obbo..obbo..",
];

const BUN: [&[u8; 14]; 3] = [
    b".....oooo.....",
    b"....ohhhho....",
    b"....ohHhho....",
];

const SIDE_PART: &[u8; 14] = b"..ohhhhhshho..";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Down,
    Up,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pose {
    Sit { typing: bool, away: bool },
    Sofa,
    Stand { face: Face },
    Walk { face: Face, step: u8 },
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    None,
    Cup { lift: bool },
    Can,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Arm {
    #[default]
    Down,
    Up,
    Wave(bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Act {
    pub lift: f32,
    pub body: f32,
    pub blink: bool,
    pub look: i8,
    pub mouth: bool,
    pub arm: Arm,
    pub hands: (bool, bool),
    pub squash: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Looks {
    pub skin: Hsla,
    pub hair: Hsla,
    pub shirt: Hsla,
    pub pants: Hsla,
    pub style: u8,
    pub tie: bool,
    pub glasses: bool,
    pub line: Hsla,
    pub hair_line: Hsla,
    pub hair_hi: Hsla,
    pub skin_dark: Hsla,
    pub lip: Hsla,
    pub shirt_dark: Hsla,
    pub pants_hi: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Front,
    Back,
    Side(bool),
}

pub fn looks_of(id: &str, role: Role, cto_shirt: Hsla) -> Looks {
    let s = motion::seed(id);
    let pick = |list: &[u32], shift: u32| hex(list[((s >> shift) as usize) % list.len()]);
    let shirt = match role {
        Role::Cto => cto_shirt,
        Role::Deep(kind) => deep_color(kind),
        Role::Staff => aura_color(id),
    };
    let skin = pick(&SKIN, 3);
    let hair = pick(&HAIR, 9);
    let pants = pick(&PANTS, 17);
    let black = hex(0x000000);
    let white = hex(0xffffff);
    Looks {
        skin,
        hair,
        shirt,
        pants,
        style: match role {
            Role::Cto => 0,
            _ => ((s >> 23) % 5) as u8,
        },
        tie: role == Role::Cto,
        glasses: matches!(role, Role::Deep(_)),
        line: hex(LINE),
        hair_line: mix(hair, black, 0.45),
        hair_hi: mix(hair, white, 0.30),
        skin_dark: mix(skin, black, 0.18),
        lip: mix(skin, hex(0x8a3b3b), 0.45),
        shirt_dark: mix(shirt, black, 0.28),
        pants_hi: mix(pants, white, 0.12),
    }
}

pub fn face_of(dx: f32, dy: f32) -> Face {
    if dx.abs() >= dy.abs() {
        if dx < 0.0 { Face::Left } else { Face::Right }
    } else if dy < 0.0 {
        Face::Up
    } else {
        Face::Down
    }
}

pub fn step_of(walked_units: f32) -> u8 {
    ((walked_units.max(0.0) / 6.0) as u32 % 4) as u8
}

pub fn head_y(pose: Pose, at: (f32, f32)) -> f32 {
    match pose {
        Pose::Sit { .. } => at.1 - SIT_TOP,
        Pose::Sofa => at.1 - SOFA_TOP,
        _ => at.1 - H,
    }
}

pub fn rows_of(pose: Pose, l: &Looks) -> (i32, i32) {
    let start = if l.style == 2 { -3 } else { 0 };
    let end = match pose {
        Pose::Sit { .. } => SIT_ROWS as i32,
        Pose::Sofa => 17,
        _ => 20,
    };
    (start, end)
}

fn view_of(pose: Pose) -> View {
    let face = match pose {
        Pose::Sit { away: true, .. } => Face::Up,
        Pose::Sit { .. } | Pose::Sofa | Pose::Stretch => Face::Down,
        Pose::Stand { face } | Pose::Walk { face, .. } => face,
    };
    match face {
        Face::Down => View::Front,
        Face::Up => View::Back,
        Face::Right => View::Side(false),
        Face::Left => View::Side(true),
    }
}

fn at(g: &Grid, r: usize, c: usize) -> u8 {
    g[r.min(g.len() - 1)][c.min(13)]
}

fn grid(v: View, pose: Pose, r: usize, c: usize) -> u8 {
    if pose == Pose::Sofa && r >= 14 {
        return at(&SOFA_LEGS, r - 14, c);
    }
    if let Pose::Walk { step, .. } = pose
        && r >= 16
    {
        let i = r - 16;
        return match v {
            View::Side(_) if step % 2 == 0 => at(&SIDE_SCISSOR, i, c),
            View::Side(_) => at(&SIDE, r, c),
            _ => match step % 4 {
                0 => at(&WALK_A, i, c),
                1 => at(&WALK_B, i, c),
                2 => at(&WALK_A, i, 13 - c),
                _ => at(&WALK_B, i, 13 - c),
            },
        };
    }
    match v {
        View::Front => at(&FRONT, r, c),
        View::Back => at(&BACK, r, c),
        View::Side(_) => at(&SIDE, r, c),
    }
}

fn styled(k: u8, v: View, l: &Looks, r: usize, c: usize) -> u8 {
    match l.style {
        1 => match v {
            View::Front | View::Back if (4..=9).contains(&r) => {
                if c == 2 || c == 11 {
                    b'h'
                } else if (c == 1 || c == 12) && k == b'.' {
                    b'o'
                } else {
                    k
                }
            }
            View::Side(_) if (7..=9).contains(&r) => {
                if c == 3 {
                    b'h'
                } else if c == 2 && k == b'.' {
                    b'o'
                } else {
                    k
                }
            }
            _ => k,
        },
        3 if v == View::Front && r == 3 => SIDE_PART[c],
        4 if r <= 2 => match k {
            b'h' | b'H' => b't',
            _ => k,
        },
        4 if r == 3 => match v {
            View::Front if (2..=11).contains(&c) => b'T',
            View::Side(_) if (7..=12).contains(&c) => b'T',
            _ => k,
        },
        _ => k,
    }
}

fn is_eye(v: View, c: i32) -> bool {
    match v {
        View::Front => c == 5 || c == 8,
        View::Side(_) => c == 9,
        View::Back => false,
    }
}

fn face_detail(k: u8, v: View, l: &Looks, act: &Act, r: usize, c: usize) -> u8 {
    if v == View::Back {
        return k;
    }
    let mut k = k;
    if r == 5 {
        if act.look != 0 {
            if is_eye(v, c as i32 - act.look as i32) {
                k = b'e';
            } else if k == b'e' {
                k = b's';
            }
        }
        if act.blink && k == b'e' {
            k = b'S';
        }
        if v == View::Front && l.glasses {
            if matches!(c, 4 | 6 | 7 | 9) {
                k = b'g';
            } else if c == 3 || c == 10 {
                k = b'l';
            }
        }
    }
    if act.mouth && r == 6 {
        let open = match v {
            View::Front => c == 6 || c == 7,
            _ => c == 9,
        };
        if open {
            k = b'M';
        }
    }
    if v == View::Front && l.tie && (10..=13).contains(&r) && (c == 6 || c == 7) {
        k = b'i';
    }
    k
}

fn swing(k: u8, r: usize, c: usize, (inner, outer): (usize, usize), hand: usize) -> u8 {
    if !(10..=13).contains(&r) || r > hand + 1 {
        return k;
    }
    let (i, o) = if r < hand {
        (b'T', b'o')
    } else if r == hand {
        (b's', b'o')
    } else {
        (b'o', b'.')
    };
    if c == inner {
        i
    } else if c == outer {
        o
    } else {
        k
    }
}

fn raised(k: u8, r: usize, c: usize, top: usize) -> u8 {
    if r < top || r > 12 || (c != 12 && c != 13) {
        return k;
    }
    let (i, o) = if r == top {
        (b's', b'o')
    } else if r < 12 {
        (b'T', b'o')
    } else {
        (b'o', b'.')
    };
    if c == 12 { i } else { o }
}

fn arms(k: u8, v: View, pose: Pose, act: &Act, r: usize, c: usize) -> u8 {
    if matches!(v, View::Side(_)) {
        return k;
    }
    if pose == Pose::Stretch {
        return match r {
            0..=2 if c == 1 || c == 12 => b's',
            3..=9 if c == 1 || c == 12 => b'T',
            10..=12 if c == 1 || c == 13 => b'.',
            10..=12 if c == 2 || c == 12 => b'o',
            _ => k,
        };
    }
    let mut k = k;
    if let Pose::Walk { step, .. } = pose
        && step % 2 == 0
    {
        let (lh, rh) = if step % 4 == 0 { (11, 13) } else { (13, 11) };
        k = swing(k, r, c, (2, 1), lh);
        k = swing(k, r, c, (12, 13), rh);
    }
    match act.arm {
        Arm::Down => k,
        Arm::Up | Arm::Wave(false) => raised(k, r, c, 7),
        Arm::Wave(true) => raised(k, r, c, 6),
    }
}

pub fn cell(pose: Pose, l: &Looks, act: &Act, row: i32, col: usize) -> u8 {
    if col >= 14 || row >= 20 {
        return b'.';
    }
    if row < 0 {
        return if l.style == 2 && row >= -3 {
            BUN[(row + 3) as usize][col]
        } else {
            b'.'
        };
    }
    let r = row as usize;
    let v = view_of(pose);
    let c = match v {
        View::Side(true) => 13 - col,
        _ => col,
    };
    let mut k = grid(v, pose, r, c);
    k = styled(k, v, l, r, c);
    k = face_detail(k, v, l, act, r, c);
    k = arms(k, v, pose, act, r, c);
    if k == b'o' && r <= 3 && l.style != 4 {
        b'O'
    } else {
        k
    }
}

fn ink(k: u8, l: &Looks) -> Option<Hsla> {
    Some(match k {
        b'o' => l.line,
        b'O' => l.hair_line,
        b'h' => l.hair,
        b'H' => l.hair_hi,
        b's' => l.skin,
        b'S' => l.skin_dark,
        b'e' | b'g' | b'b' => hex(EYE),
        b'm' => l.lip,
        b'M' => hex(MOUTH_OPEN),
        b't' => l.shirt,
        b'T' => l.shirt_dark,
        b'c' => hex(COLLAR),
        b'k' => hex(BELT),
        b'i' => hex(TIE),
        b'p' => l.pants,
        b'P' => l.pants_hi,
        b'l' => hex(LENS),
        _ => return None,
    })
}

struct Row<'a> {
    pose: Pose,
    l: &'a Looks,
    act: &'a Act,
    left: f32,
    fade: f32,
}

fn paint_row(b: &mut Brush, w: &Row, row: i32, y: f32) {
    let mut start = 0;
    let mut run = cell(w.pose, w.l, w.act, row, 0);
    for col in 1..=14 {
        let k = if col < 14 { cell(w.pose, w.l, w.act, row, col) } else { 0 };
        if k != run {
            if let Some(c) = ink(run, w.l) {
                b.rect((w.left + start as f32, y, (col - start) as f32, 1.0), c, w.fade);
            }
            start = col;
            run = k;
        }
    }
}

fn shadow(b: &mut Brush, (x, y): (f32, f32), bob: bool, fade: f32) {
    let n = if bob { 1.0 } else { 0.0 };
    let c = hex(SHADOW);
    b.rect((x - 6.0 + n, y - 1.0, 12.0 - 2.0 * n, 2.0), c, 0.10 * fade);
    b.rect((x - 5.0 + n, y - 2.0, 10.0 - 2.0 * n, 1.0), c, 0.06 * fade);
    b.rect((x - 4.0 + n, y + 1.0, 8.0 - 2.0 * n, 1.0), c, 0.08 * fade);
}

pub fn paint(b: &mut Brush, (x, y): (f32, f32), l: &Looks, pose: Pose, act: &Act, fade: f32) {
    let top = head_y(pose, (x, y)) + act.body;
    let bob = matches!(pose, Pose::Walk { step, .. } if step % 2 == 1);
    if matches!(pose, Pose::Stand { .. } | Pose::Walk { .. } | Pose::Stretch) {
        shadow(b, (x, y), bob, fade);
    }
    let w = Row {
        pose,
        l,
        act,
        left: x - W / 2.0,
        fade,
    };
    let (start, end) = rows_of(pose, l);
    let upper = if matches!(pose, Pose::Stand { .. }) { 13 } else { 8 };
    for row in start..end {
        if act.squash && row == 8 {
            continue;
        }
        let mut dy = 0.0;
        if row <= upper {
            dy += act.lift;
        }
        if row <= 8 && act.squash {
            dy += 1.0;
        }
        if bob && row <= 15 {
            dy -= 1.0;
        }
        paint_row(b, &w, row, top + row as f32 + dy);
        if bob && row == 15 {
            paint_row(b, &w, row, top + row as f32);
        }
        if act.lift < 0.0 && row == upper {
            let fill = if bob { -1.0 } else { 0.0 };
            paint_row(b, &w, row, top + row as f32 + fill);
        }
    }
}

fn cup(b: &mut Brush, (x, y): (f32, f32)) {
    b.solid((x - 1.0, y - 1.0, 5.0, 5.0), LINE);
    b.solid((x, y, 3.0, 3.0), 0xffffff);
    b.solid((x, y + 1.0, 3.0, 1.0), 0xc9a27a);
}

pub fn paint_prop(b: &mut Brush, (x, y): (f32, f32), pose: Pose, prop: Prop, act: &Act, now: u64) {
    let base = head_y(pose, (x, y)) + H + act.body;
    match prop {
        Prop::None => {}
        Prop::Cup { lift: false } => {
            cup(b, (x + 4.0, base - 11.0));
            let k = ((now / 300) % 3) as f32;
            let a = 0.5 - 0.15 * k;
            b.rect((x + 5.0, base - 13.0 - k, 1.0, 1.0), hex(0xffffff), a);
            b.rect((x + 6.0, base - 14.0 - k, 1.0, 1.0), hex(0xffffff), a);
        }
        Prop::Cup { lift: true } => cup(b, (x + 1.0, base - 15.0)),
        Prop::Can => {
            b.solid((x + 3.0, base - 13.0, 5.0, 5.0), LINE);
            b.solid((x + 4.0, base - 12.0, 3.0, 3.0), 0x7aa6c9);
            b.solid((x + 7.0, base - 13.0, 2.0, 1.0), 0x7aa6c9);
            for i in 0..3u64 {
                let dy = ((now / 120 + 2 * i) % 6) as f32;
                b.solid((x + 9.0, base - 12.0 + dy, 1.0, 1.0), 0x8cc4ee);
            }
        }
    }
}

pub fn typing_hands(b: &mut Brush, (x, y): (f32, f32), l: &Looks, act: &Act) {
    for (hx, up) in [(x - 5.0, act.hands.0), (x + 3.0, act.hands.1)] {
        let hy = y - 1.0 - if up { 1.0 } else { 0.0 };
        b.rect((hx, hy + 1.0, 2.0, 1.0), l.line, 1.0);
        b.rect((hx, hy, 2.0, 1.0), l.skin, 1.0);
    }
}

pub fn paint_sip(b: &mut Brush, (x, y): (f32, f32), pose: Pose, act: &Act, k: f32) {
    let base = head_y(pose, (x, y)) + H + act.body;
    let k = k.clamp(0.0, 1.0);
    let cx = (x + 4.0) + (x + 1.0 - (x + 4.0)) * k;
    let cy = (base - 11.0) + (base - 15.0 - (base - 11.0)) * k;
    cup(b, (cx.round(), cy.round()));
}
