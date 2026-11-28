use super::sprite::{Act, Pose};

pub fn seed(id: &str) -> u64 {
    let mut s: u64 = 1_469_598_103_934_665_603;
    for ch in id.chars() {
        s = (s ^ ch as u64).wrapping_mul(1_099_511_628_211);
    }
    s
}

pub fn hash(seed: u64, n: u64) -> u64 {
    let mut x = seed ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn clamp(k: f32) -> f32 {
    k.clamp(0.0, 1.0)
}

pub fn in_cubic(k: f32) -> f32 {
    let k = clamp(k);
    k * k * k
}

pub fn out_cubic(k: f32) -> f32 {
    let k = clamp(k);
    1.0 - (1.0 - k).powi(3)
}

pub fn in_out_cubic(k: f32) -> f32 {
    let k = clamp(k);
    if k < 0.5 {
        4.0 * k * k * k
    } else {
        1.0 - (-2.0 * k + 2.0).powi(3) / 2.0
    }
}

pub fn out_back(k: f32, s: f32) -> f32 {
    let u = clamp(k) - 1.0;
    1.0 + (s + 1.0) * u * u * u + s * u * u
}

pub fn trip(t_ms: f32, total_ms: f32) -> f32 {
    if total_ms <= 0.0 {
        return 1.0;
    }
    let total = total_ms;
    let t = t_ms.clamp(0.0, total);
    let r = (total / 3.0).min(220.0);
    let v = 1.0 / (total - r);
    let k = if t < r {
        0.5 * v * t * t / r
    } else if t <= total - r {
        v * (t - r / 2.0)
    } else {
        1.0 - 0.5 * v * (total - t) * (total - t) / r
    };
    clamp(k)
}

pub fn time_of(k: f32, total_ms: f32) -> f32 {
    if total_ms <= 0.0 {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0f32, total_ms);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if trip(mid, total_ms) < k {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

pub fn sip_k(t: u64, len: u64) -> f32 {
    if t >= len || len == 0 {
        return 0.0;
    }
    let up = 220.min(len / 2).max(1);
    let down = 280.min(len / 2).max(1);
    if t < up {
        out_cubic(t as f32 / up as f32)
    } else if t + down > len {
        1.0 - in_out_cubic((t + down - len) as f32 / down as f32)
    } else {
        1.0
    }
}

pub fn hands_of(seed: u64, now: u64) -> (bool, bool) {
    let up = |hand: u64| hash(seed, now / 90 + hand).is_multiple_of(3);
    (up(0), up(1))
}

fn window(seed: u64, now: u64, period: u64, len: u64) -> bool {
    (now + seed % period) % period < len
}

pub fn act_of(seed: u64, pose: Pose, now: u64) -> Act {
    let mut act = Act::default();
    let glance = 7000 + (seed >> 9) % 4000;
    if window(seed >> 3, now, glance, 900) {
        let round = (now + (seed >> 3) % glance) / glance;
        act.look = if hash(seed, round) & 1 == 0 { 1 } else { -1 };
    }
    if pose == Pose::Sofa {
        return act;
    }
    let blink = 3200 + (seed >> 5) % 2000;
    act.blink = window(seed >> 7, now, blink, 150);
    if matches!(pose, Pose::Sit { .. } | Pose::Stand { .. }) {
        let breath = 2600 + seed % 800;
        if window(seed, now, breath, breath * 42 / 100) {
            act.lift = -1.0;
        }
    }
    act
}

pub fn merge(idle: Act, own: Act) -> Act {
    Act {
        lift: if own.lift != 0.0 { own.lift } else { idle.lift },
        body: own.body + idle.body,
        blink: idle.blink || own.blink,
        look: if own.look != 0 { own.look } else { idle.look },
        ..own
    }
}
