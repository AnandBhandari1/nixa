use gpui::{Bounds, Font, Global, Pixels, Point, Size, Window, WindowId, px};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HIDDEN_AFTER: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Default)]
pub struct FrameClock(Arc<Mutex<HashMap<WindowId, Instant>>>);

impl Global for FrameClock {}

pub fn mark_frame(clock: &FrameClock, window: WindowId, at: Instant) {
    if let Ok(mut frames) = clock.0.lock() {
        let last = frames.entry(window).or_insert(at);
        if *last < at {
            *last = at;
        }
    }
}

pub fn left_out(clock: &FrameClock, seen: Option<(WindowId, Instant)>) -> bool {
    let Some((window, at)) = seen else {
        return false;
    };
    clock
        .0
        .lock()
        .ok()
        .and_then(|frames| frames.get(&window).copied())
        .is_some_and(|last| last > at + HIDDEN_AFTER)
}

#[derive(Debug, Default)]
pub struct PaintStats {
    frames: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaintSummary {
    pub frames: u64,
    pub total: Duration,
    pub max: Duration,
}

pub fn note_paint(stats: &PaintStats, took: Duration) {
    let ns = took.as_nanos().min(u64::MAX as u128) as u64;
    stats.frames.fetch_add(1, Ordering::Relaxed);
    stats.total_ns.fetch_add(ns, Ordering::Relaxed);
    stats.max_ns.fetch_max(ns, Ordering::Relaxed);
}

pub fn paint_summary(stats: &PaintStats) -> PaintSummary {
    PaintSummary {
        frames: stats.frames.load(Ordering::Relaxed),
        total: Duration::from_nanos(stats.total_ns.load(Ordering::Relaxed)),
        max: Duration::from_nanos(stats.max_ns.load(Ordering::Relaxed)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub cell_w: Pixels,
    pub line_h: Pixels,
    pub font_size: Pixels,
}

pub const LINE_FACTOR: f32 = 1.3;

pub fn measure(window: &Window, font: &Font, font_size: Pixels) -> Metrics {
    let ts = window.text_system();
    let id = ts.resolve_font(font);
    let cell_w = ts
        .advance(id, font_size, 'm')
        .map(|s| s.width)
        .unwrap_or(font_size * 0.6);
    Metrics {
        cell_w: if cell_w > px(0.0) {
            cell_w
        } else {
            font_size * 0.6
        },
        line_h: (font_size * LINE_FACTOR).round(),
        font_size,
    }
}

pub fn grid_size(size: Size<Pixels>, m: &Metrics) -> (u16, u16) {
    let cols = (size.width / m.cell_w).floor().clamp(2.0, 1000.0) as u16;
    let rows = (size.height / m.line_h).floor().clamp(1.0, 1000.0) as u16;
    (rows, cols)
}

pub fn cell_at(
    pos: Point<Pixels>,
    bounds: &Bounds<Pixels>,
    m: &Metrics,
    rows: u16,
    cols: u16,
) -> (u16, u16) {
    let x = ((pos.x - bounds.origin.x) / m.cell_w).floor().max(0.0) as u16;
    let y = ((pos.y - bounds.origin.y) / m.line_h).floor().max(0.0) as u16;
    (y.min(rows.saturating_sub(1)), x.min(cols.saturating_sub(1)))
}

pub fn cell_origin(bounds: &Bounds<Pixels>, m: &Metrics, row: u16, col: u16) -> Point<Pixels> {
    Point {
        x: bounds.origin.x + m.cell_w * col as f32,
        y: bounds.origin.y + m.line_h * row as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_left_out_of_later_frames_is_hidden() {
        let clock = FrameClock::default();
        let (a, b) = (WindowId::from(1u64), WindowId::from(2u64));
        let t0 = Instant::now();
        assert!(!left_out(&clock, None));
        mark_frame(&clock, a, t0);
        assert!(!left_out(&clock, Some((a, t0))));
        mark_frame(&clock, a, t0 + Duration::from_millis(10));
        assert!(!left_out(&clock, Some((a, t0))));
        mark_frame(&clock, b, t0 + Duration::from_secs(5));
        assert!(!left_out(&clock, Some((a, t0))));
        mark_frame(&clock, a, t0 + HIDDEN_AFTER + Duration::from_millis(20));
        assert!(left_out(&clock, Some((a, t0))));
        mark_frame(&clock, a, t0);
        assert!(left_out(&clock, Some((a, t0))));
    }

    #[test]
    fn paint_stats_add_up() {
        let stats = PaintStats::default();
        note_paint(&stats, Duration::from_millis(2));
        note_paint(&stats, Duration::from_millis(5));
        let s = paint_summary(&stats);
        assert_eq!(s.frames, 2);
        assert_eq!(s.total, Duration::from_millis(7));
        assert_eq!(s.max, Duration::from_millis(5));
    }
}
