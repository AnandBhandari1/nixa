use std::fs;
use std::io::Read;

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32)
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32)
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

fn le24(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, 0]))
}

fn png(b: &[u8]) -> Option<(u32, u32)> {
    (b.starts_with(b"\x89PNG\r\n\x1a\n") && b.get(12..16)? == b"IHDR").then_some(())?;
    Some((be32(b, 16)?, be32(b, 20)?))
}

fn gif(b: &[u8]) -> Option<(u32, u32)> {
    (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")).then_some(())?;
    Some((le16(b, 6)?, le16(b, 8)?))
}

fn bmp(b: &[u8]) -> Option<(u32, u32)> {
    b.starts_with(b"BM").then_some(())?;
    let w = i32::from_le_bytes(b.get(18..22)?.try_into().ok()?);
    let h = i32::from_le_bytes(b.get(22..26)?.try_into().ok()?);
    Some((w.unsigned_abs(), h.unsigned_abs()))
}

fn ico(b: &[u8]) -> Option<(u32, u32)> {
    b.starts_with(&[0, 0, 1, 0]).then_some(())?;
    let side = |v: u8| if v == 0 { 256 } else { v as u32 };
    Some((side(*b.get(6)?), side(*b.get(7)?)))
}

fn jpeg(b: &[u8]) -> Option<(u32, u32)> {
    b.starts_with(&[0xff, 0xd8]).then_some(())?;
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        let len = be16(b, i + 2)? as usize;
        let sof = (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker);
        if sof {
            return Some((be16(b, i + 7)?, be16(b, i + 5)?));
        }
        i += 2 + len;
    }
    None
}

fn webp(b: &[u8]) -> Option<(u32, u32)> {
    (b.get(0..4)? == b"RIFF" && b.get(8..12)? == b"WEBP").then_some(())?;
    match b.get(12..16)? {
        b"VP8 " => Some((le16(b, 26)? & 0x3fff, le16(b, 28)? & 0x3fff)),
        b"VP8L" => {
            let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
            Some(((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le24(b, 24)? + 1, le24(b, 27)? + 1)),
        _ => None,
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        let before = tag[..at].chars().next_back();
        let rest = tag[at + name.len()..].trim_start();
        if before.is_some_and(char::is_whitespace) && let Some(rest) = rest.strip_prefix('=') {
            let rest = rest.trim_start();
            let q = rest.chars().next()?;
            if q == '"' || q == '\'' {
                let body = &rest[1..];
                return body.find(q).map(|end| body[..end].to_string());
            }
        }
        from = at + name.len();
    }
    None
}

fn number(v: &str) -> Option<f64> {
    let v = v.trim();
    let v = v.strip_suffix("px").unwrap_or(v);
    v.parse::<f64>().ok().filter(|n| *n > 0.0)
}

fn svg(b: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(b).ok()?;
    let start = text.find("<svg")?;
    let end = start + text[start..].find('>')?;
    let tag = &text[start..end];
    if let (Some(w), Some(h)) = (attr(tag, "width").and_then(|v| number(&v)), attr(tag, "height").and_then(|v| number(&v))) {
        return Some((w.round() as u32, h.round() as u32));
    }
    let view = attr(tag, "viewBox")?;
    let parts: Vec<f64> = view.split([' ', ',']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    (parts.len() == 4 && parts[2] > 0.0 && parts[3] > 0.0).then(|| (parts[2].round() as u32, parts[3].round() as u32))
}

pub fn dims(bytes: &[u8]) -> Option<(u32, u32)> {
    png(bytes).or_else(|| gif(bytes)).or_else(|| jpeg(bytes)).or_else(|| webp(bytes)).or_else(|| bmp(bytes)).or_else(|| ico(bytes)).or_else(|| svg(bytes))
}

pub fn dims_of_file(path: &str) -> Option<(u32, u32)> {
    let mut f = fs::File::open(path).ok()?;
    let mut head = Vec::new();
    f.by_ref().take(256 * 1024).read_to_end(&mut head).ok()?;
    dims(&head)
}

pub fn fit(img: (u32, u32), box_w: f32, box_h: f32) -> (f32, f32) {
    let (w, h) = (img.0.max(1) as f32, img.1.max(1) as f32);
    let scale = ((box_w - 32.0) / w).min((box_h - 32.0) / h).clamp(0.01, 1.0);
    ((w * scale).floor().max(1.0), (h * scale).floor().max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_gif_bmp_ico() {
        let mut p = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        p.extend(640u32.to_be_bytes());
        p.extend(480u32.to_be_bytes());
        assert_eq!(dims(&p), Some((640, 480)));
        let mut g = b"GIF89a".to_vec();
        g.extend(20u16.to_le_bytes());
        g.extend(10u16.to_le_bytes());
        assert_eq!(dims(&g), Some((20, 10)));
        let mut b = b"BM".to_vec();
        b.resize(18, 0);
        b.extend(7i32.to_le_bytes());
        b.extend((-9i32).to_le_bytes());
        assert_eq!(dims(&b), Some((7, 9)));
        assert_eq!(dims(&[0, 0, 1, 0, 1, 0, 0, 32]), Some((256, 32)));
    }

    #[test]
    fn jpeg_frame() {
        let j = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 50, 0, 100, 3, 0, 0, 0];
        assert_eq!(dims(&j), Some((100, 50)));
    }

    #[test]
    fn svg_size_and_view_box() {
        assert_eq!(dims(br#"<svg xmlns="x" width="120px" height='40'>"#), Some((120, 40)));
        assert_eq!(dims(br#"<?xml?><svg viewBox="0 0 24 12" stroke-width="2">"#), Some((24, 12)));
        assert_eq!(dims(b"not an image"), None);
    }

    #[test]
    fn fit_never_grows() {
        assert_eq!(fit((100, 50), 1000.0, 1000.0), (100.0, 50.0));
        assert_eq!(fit((1000, 500), 532.0, 1000.0), (500.0, 250.0));
    }
}
