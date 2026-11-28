pub const SAMPLE_RATE: u32 = 16_000;
pub const VAD_WINDOW: usize = 512;

pub fn i16_to_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter().map(|&s| s as f32 / 32768.0).collect()
}

pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| {
            let v = s.clamp(-1.0, 1.0);
            if v < 0.0 {
                (v * 32768.0).round() as i16
            } else {
                (v * 32767.0).round() as i16
            }
        })
        .collect()
}

pub fn to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

pub fn level(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f64 = pcm.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
    ((sum / pcm.len() as f64).sqrt() as f32).min(1.0)
}

pub struct Resampler {
    from: u32,
    to: u32,
    pos: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Resampler { from, to, pos: 1.0, last: 0.0 }
    }

    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if self.from == self.to || input.is_empty() {
            return input.to_vec();
        }
        let step = self.from as f64 / self.to as f64;
        let total = input.len() + 1;
        let at = |i: usize| if i == 0 { self.last } else { input[i - 1] };
        let mut out = Vec::with_capacity((input.len() as f64 / step) as usize + 2);
        while (self.pos.floor() as usize) + 1 < total {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(at(i) * (1.0 - frac) + at(i + 1) * frac);
            self.pos += step;
        }
        self.pos -= (total - 1) as f64;
        self.last = input[input.len() - 1];
        out
    }
}

pub struct Chunker {
    window: usize,
    pending: Vec<f32>,
}

impl Chunker {
    pub fn new(window: usize) -> Self {
        Chunker { window: window.max(1), pending: Vec::new() }
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let full = self.pending.len() / self.window * self.window;
        let out = self.pending[..full].chunks(self.window).map(|c| c.to_vec()).collect();
        self.pending.drain(..full);
        out
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn take_padded(&mut self) -> Option<Vec<f32>> {
        if self.pending.is_empty() {
            return None;
        }
        let mut last = std::mem::take(&mut self.pending);
        last.resize(self.window, 0.0);
        Some(last)
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunker_splits_into_full_windows_and_keeps_rest() {
        let mut c = Chunker::new(512);
        assert!(c.push(&[0.1; 300]).is_empty());
        assert_eq!(c.pending(), 300);
        let out = c.push(&[0.2; 1000]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|w| w.len() == 512));
        assert_eq!(out[0][299], 0.1);
        assert_eq!(out[0][300], 0.2);
        assert_eq!(c.pending(), 1300 - 1024);
    }

    #[test]
    fn chunker_pads_the_last_window_with_silence() {
        let mut c = Chunker::new(4);
        c.push(&[1.0; 6]);
        let last = c.take_padded().unwrap();
        assert_eq!(last, vec![1.0, 1.0, 0.0, 0.0]);
        assert!(c.take_padded().is_none());
    }

    #[test]
    fn chunker_exact_multiple_leaves_nothing() {
        let mut c = Chunker::new(512);
        assert_eq!(c.push(&[0.0; 1536]).len(), 3);
        assert_eq!(c.pending(), 0);
    }

    #[test]
    fn i16_round_trip_keeps_values() {
        let pcm = vec![0i16, 1, -1, 16384, -16384, i16::MAX, i16::MIN];
        let back = f32_to_i16(&i16_to_f32(&pcm));
        for (a, b) in pcm.iter().zip(back.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "{a} {b}");
        }
        assert_eq!(f32_to_i16(&[2.0, -2.0]), vec![i16::MAX, i16::MIN]);
    }

    #[test]
    fn mono_averages_channels() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(to_mono(&[0.3, 0.4], 1), vec![0.3, 0.4]);
    }

    #[test]
    fn resampler_gives_the_right_length_across_chunks() {
        let mut r = Resampler::new(48_000, 16_000);
        let mut total = 0;
        for _ in 0..100 {
            total += r.push(&[0.5; 480]).len();
        }
        assert!((total as i64 - 16_000).abs() <= 2, "{total}");
    }

    #[test]
    fn resampler_upsamples_and_interpolates() {
        let mut r = Resampler::new(8_000, 16_000);
        let out = r.push(&[0.0, 1.0, 0.0, 1.0]);
        assert_eq!(out.len(), 6);
        assert!((out[0] - 0.0).abs() < 1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);
        assert!((out[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn level_of_silence_is_zero_and_full_scale_is_one() {
        assert_eq!(level(&[0; 100]), 0.0);
        assert!(level(&[i16::MIN; 100]) > 0.99);
        assert_eq!(level(&[]), 0.0);
    }
}
