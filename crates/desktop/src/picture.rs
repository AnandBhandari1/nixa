use base64::Engine;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

#[cfg(target_os = "macos")]
const NO_FOLLOW: i32 = 0x0100;
#[cfg(not(target_os = "macos"))]
const NO_FOLLOW: i32 = 0o400000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub media_type: &'static str,
    pub data: String,
    pub bytes: u64,
}

const PNG_HEAD: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 {
        return None;
    }
    if bytes.starts_with(&PNG_HEAD) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || !head.starts_with(&PNG_HEAD) || &head[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    (width > 0 && height > 0).then_some((width, height))
}

pub fn open_plain(path: &Path) -> Result<File, String> {
    let file = std::fs::OpenOptions::new().read(true).custom_flags(NO_FOLLOW).open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "The capture file is gone. Take a new shot.".to_string()
        } else {
            format!("The capture cannot be opened ({e}). Take a new shot.")
        }
    })?;
    let info = file.metadata().map_err(|e| format!("The capture cannot be read ({e})."))?;
    if !info.is_file() {
        return Err("The capture is not a regular file.".into());
    }
    Ok(file)
}

pub fn png_info(path: &Path) -> Result<PngInfo, String> {
    let mut file = open_plain(path)?;
    let bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let mut head = [0u8; 24];
    file.read_exact(&mut head).map_err(|_| "The capture is not a PNG picture.".to_string())?;
    let (width, height) = png_size(&head).ok_or("The capture is not a PNG picture with a size.")?;
    Ok(PngInfo { width, height, bytes })
}

pub fn check_size(size: u64) -> Result<(), String> {
    if size == 0 {
        return Err("The capture is empty. Take a new shot.".into());
    }
    if size > MAX_IMAGE_BYTES {
        return Err(format!(
            "The capture is too large to send: {:.1} MB, the limit is {} MB. Capture a smaller window.",
            size as f64 / 1_048_576.0,
            MAX_IMAGE_BYTES / 1_048_576
        ));
    }
    Ok(())
}

pub fn check_image(bytes: &[u8]) -> Result<Picture, String> {
    check_size(bytes.len() as u64)?;
    let media_type = image_type(bytes).ok_or("The capture is not a PNG, JPEG, WebP or GIF picture, so it was not sent.")?;
    Ok(Picture { media_type, data: base64::engine::general_purpose::STANDARD.encode(bytes), bytes: bytes.len() as u64 })
}

pub fn read_picture(path: &Path) -> Result<Picture, String> {
    let mut file = open_plain(path)?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    check_size(size)?;
    let mut bytes = Vec::with_capacity(size as usize);
    file.by_ref().take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("The capture cannot be read ({e})."))?;
    if bytes.len() as u64 != size {
        return Err("The capture changed while it was read. Take a new shot.".into());
    }
    check_image(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn tiny_png(width: u32, height: u32) -> Vec<u8> {
        let mut out = PNG_HEAD.to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    #[test]
    fn knows_picture_types_from_the_first_bytes() {
        assert_eq!(image_type(&tiny_png(3, 4)), Some("image/png"));
        assert_eq!(image_type(&[0xff, 0xd8, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some("image/jpeg"));
        assert_eq!(image_type(b"GIF89a______"), Some("image/gif"));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_type(b"<html>hello</html>"), None);
        assert_eq!(image_type(b"short"), None);
    }

    #[test]
    fn reads_the_png_size_from_its_header() {
        assert_eq!(png_size(&tiny_png(3200, 1628)), Some((3200, 1628)));
        assert_eq!(png_size(&tiny_png(0, 5)), None);
        assert_eq!(png_size(b"not a png at all, no header here"), None);
    }

    #[test]
    fn refuses_empty_big_and_linked_files() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.png");
        std::fs::write(&empty, b"").unwrap();
        assert!(read_picture(&empty).unwrap_err().contains("empty"));
        let big = dir.path().join("big.png");
        std::fs::write(&big, vec![0u8; (MAX_IMAGE_BYTES + 1) as usize]).unwrap();
        assert!(read_picture(&big).unwrap_err().contains("too large"));
        let real = dir.path().join("real.png");
        std::fs::write(&real, tiny_png(10, 20)).unwrap();
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(read_picture(&link).is_err());
        assert!(png_info(&link).is_err());
        let info = png_info(&real).unwrap();
        assert_eq!((info.width, info.height), (10, 20));
        let picture = read_picture(&real).unwrap();
        assert_eq!(picture.media_type, "image/png");
        assert!(!picture.data.is_empty());
        let text = dir.path().join("text.png");
        std::fs::write(&text, b"this is plain text, not a picture").unwrap();
        assert!(read_picture(&text).unwrap_err().contains("not a PNG"));
        assert!(read_picture(&dir.path().join("gone.png")).unwrap_err().contains("gone"));
    }
}
