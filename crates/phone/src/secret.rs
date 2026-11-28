use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::{SecureRandom, SystemRandom};

pub fn random_bytes(n: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; n];
    SystemRandom::new().fill(&mut bytes).map_err(|_| anyhow!("no random bytes from the system"))?;
    Ok(bytes)
}

pub fn random_key() -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes(32)?))
}

pub fn random_id() -> Result<String> {
    Ok(hex(&random_bytes(4)?))
}

pub fn sha256(text: &str) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, text.as_bytes()).as_ref().to_vec()
}

pub fn sha256_bytes(bytes: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().to_vec()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn is_key(text: &str) -> bool {
    text.len() == 43 && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key.trim() == name && !key.trim().is_empty()).then(|| value.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_ids_have_the_right_shape() {
        let key = random_key().unwrap();
        assert!(is_key(&key));
        assert_ne!(key, random_key().unwrap());
        assert!(is_hex(&random_id().unwrap(), 8));
        assert!(!is_key("short"));
        assert!(!is_key(&format!("{}!", "a".repeat(42))));
    }

    #[test]
    fn hashes_compare_in_full() {
        assert!(same(&sha256("a"), &sha256("a")));
        assert!(!same(&sha256("a"), &sha256("b")));
        assert!(!same(b"ab", b"abc"));
        assert_eq!(unhex(&hex(&[0, 15, 255])).unwrap(), vec![0, 15, 255]);
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
    }

    #[test]
    fn cookie_is_found_by_exact_name() {
        let header = "other=1; nixa_phone=ab12cd34.key ; x=2";
        assert_eq!(cookie_value(header, "nixa_phone").as_deref(), Some("ab12cd34.key"));
        assert_eq!(cookie_value("nixa_phone_x=1", "nixa_phone"), None);
        assert_eq!(cookie_value("", "nixa_phone"), None);
    }
}
