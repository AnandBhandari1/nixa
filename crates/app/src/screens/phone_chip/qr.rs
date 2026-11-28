use gpui::prelude::*;
use gpui::{AnyElement, Bounds, Hsla, canvas, div, fill, point, px, size};
use qrcode::{Color, EcLevel, QrCode};

pub fn make(text: &str) -> Vec<String> {
    let Ok(code) = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M) else {
        return Vec::new();
    };
    let width = code.width();
    code.to_colors()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|c| if *c == Color::Dark { '1' } else { '0' })
                .collect()
        })
        .collect()
}

pub fn runs(rows: &[String]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let bytes = row.as_bytes();
        let mut x = 0;
        while x < bytes.len() {
            if bytes[x] != b'1' {
                x += 1;
                continue;
            }
            let start = x;
            while x < bytes.len() && bytes[x] == b'1' {
                x += 1;
            }
            out.push((start, y, x - start));
        }
    }
    out
}

pub fn cell(n: usize, box_size: f32) -> f32 {
    (box_size / (n + 2) as f32).floor().max(2.0)
}

pub fn qr_box(id: &'static str, rows: &[String], box_size: f32) -> AnyElement {
    let n = rows.len();
    let c = cell(n, box_size);
    let full = c * (n + 2) as f32;
    let dark: Vec<(usize, usize, usize)> = runs(rows);
    let black = Hsla::black();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .relative()
        .flex_none()
        .size(px(full))
        .rounded(px(6.))
        .bg(Hsla::white())
        .child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    for (x, y, len) in &dark {
                        let at = point(
                            b.origin.x + px((x + 1) as f32 * c),
                            b.origin.y + px((y + 1) as f32 * c),
                        );
                        window.paint_quad(fill(
                            Bounds::new(at, size(px(*len as f32 * c), px(c))),
                            black,
                        ));
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}
