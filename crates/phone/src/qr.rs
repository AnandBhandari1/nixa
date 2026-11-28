use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use qrcode::{Color, EcLevel, QrCode};

pub fn root_pin(root_der: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(crate::secret::sha256_bytes(root_der))
}

pub fn pair_text(address: &str, port: u16, key: &str, root_der: &[u8]) -> String {
    format!("https://{address}:{port}/k/{key}#r={}", root_pin(root_der))
}

pub fn pin_of(text: &str) -> Option<&str> {
    let (_, pin) = text.split_once("#r=")?;
    (pin.len() == 43 && pin.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')).then_some(pin)
}

pub fn qr_rows(text: &str) -> Result<Vec<String>> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).map_err(|e| anyhow!("QR code failed: {e}"))?;
    let width = code.width();
    let rows: Vec<String> = code
        .to_colors()
        .chunks(width)
        .map(|row| row.iter().map(|c| if *c == Color::Dark { '1' } else { '0' }).collect())
        .collect();
    if rows.len() < 21 || rows.iter().any(|row| row.len() != rows.len()) {
        return Err(anyhow!("QR code failed: bad size"));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_square_and_have_finder_marks() {
        let rows = qr_rows("https://192.168.1.5:7430/k/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ").unwrap();
        assert!(rows.len() >= 21);
        assert!(rows.iter().all(|row| row.len() == rows.len() && row.bytes().all(|b| b == b'0' || b == b'1')));
        assert_eq!(&rows[0][..7], "1111111");
        assert_eq!(&rows[1][..7], "1000001");
        assert_eq!(&rows[0][rows.len() - 7..], "1111111");
    }

    #[test]
    fn pair_text_carries_the_root_pin() {
        let key = "k".repeat(43);
        let text = pair_text("192.168.1.5", 7430, &key, b"root");
        let pin = pin_of(&text).unwrap();
        assert_eq!(pin.len(), 43);
        assert_eq!(pin, root_pin(b"root"));
        assert_ne!(pin, root_pin(b"other"));
        assert_eq!(text, format!("https://192.168.1.5:7430/k/{key}#r={pin}"));
        assert!(qr_rows(&text).is_ok());
        assert_eq!(pin_of("https://192.168.1.5:7430/k/x"), None);
        assert_eq!(pin_of("https://h/k/x#r=short"), None);
    }
}
