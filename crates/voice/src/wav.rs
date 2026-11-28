use anyhow::{Result, bail};

pub struct Wav {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn parse_wav(bytes: &[u8]) -> Result<Wav> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a wav file");
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " {
            if body + 16 > bytes.len() {
                bail!("short fmt chunk");
            }
            let mut tag = u16_at(bytes, body);
            if tag == 0xFFFE && size >= 26 && body + 26 <= bytes.len() {
                tag = u16_at(bytes, body + 24);
            }
            format = Some((tag, u16_at(bytes, body + 2), u32_at(bytes, body + 4), u16_at(bytes, body + 14)));
        } else if id == b"data" {
            let Some((tag, channels, rate, bits)) = format else { bail!("data before fmt") };
            let end = if size == 0 || body + size > bytes.len() { bytes.len() } else { body + size };
            let data = &bytes[body..end];
            let samples = decode(data, tag, bits)?;
            return Ok(Wav { rate, channels: channels.max(1), samples });
        }
        at = body + size + (size & 1);
    }
    bail!("no data chunk")
}

fn decode(data: &[u8], tag: u16, bits: u16) -> Result<Vec<f32>> {
    Ok(match (tag, bits) {
        (1, 16) => data.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes(*c) as f32 / 2_147_483_648.0)
            .collect(),
        (1, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (3, 32) => data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        _ => bail!("wav format {tag} with {bits} bits is not supported"),
    })
}

pub fn wav_bytes(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_16_bit() {
        let pcm = vec![0i16, 1000, -1000, i16::MAX];
        let wav = parse_wav(&wav_bytes(&pcm, 24_000)).unwrap();
        assert_eq!(wav.rate, 24_000);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.samples.len(), 4);
        assert!((wav.samples[1] - 1000.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn skips_unknown_chunks() {
        let mut bytes = wav_bytes(&[5, 6], 16_000);
        let extra = [b"LIST".as_slice(), &3u32.to_le_bytes(), &[1, 2, 3, 0]].concat();
        bytes.splice(36..36, extra);
        let wav = parse_wav(&bytes).unwrap();
        assert_eq!(wav.samples.len(), 2);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse_wav(b"hello world!").is_err());
    }
}
