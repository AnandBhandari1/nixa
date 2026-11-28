use crate::pcm::{Chunker, SAMPLE_RATE, VAD_WINDOW, f32_to_i16, i16_to_f32};
use anyhow::{Result, bail};
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VadOut {
    Start,
    End,
    Phrase(Vec<i16>),
}

pub fn edge(was: bool, now: bool) -> Option<Edge> {
    match (was, now) {
        (false, true) => Some(Edge::Start),
        (true, false) => Some(Edge::End),
        _ => None,
    }
}

pub struct Vad {
    detector: VoiceActivityDetector,
    chunker: Chunker,
    speaking: bool,
}

impl Vad {
    pub fn new(model: &Path, silence_ms: u32) -> Result<Self> {
        if !model.exists() {
            bail!("vad model missing: {}", model.display());
        }
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model.to_string_lossy().into_owned()),
                threshold: 0.5,
                min_speech_duration: 0.25,
                min_silence_duration: silence_ms as f32 / 1000.0,
                max_speech_duration: 20.0,
                window_size: VAD_WINDOW as i32,
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
            ..Default::default()
        };
        let Some(detector) = VoiceActivityDetector::create(&config, 60.0) else {
            bail!("could not start the vad with {}", model.display());
        };
        Ok(Vad { detector, chunker: Chunker::new(VAD_WINDOW), speaking: false })
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    fn track(&mut self, out: &mut Vec<VadOut>) {
        let now = self.detector.detected();
        match edge(self.speaking, now) {
            Some(Edge::Start) => out.push(VadOut::Start),
            Some(Edge::End) => out.push(VadOut::End),
            None => {}
        }
        self.speaking = now;
    }

    fn drain(&mut self, out: &mut Vec<VadOut>) {
        while !self.detector.is_empty() {
            if let Some(segment) = self.detector.front()
                && !segment.samples().is_empty()
            {
                out.push(VadOut::Phrase(f32_to_i16(segment.samples())));
            }
            self.detector.pop();
        }
    }

    pub fn push(&mut self, frame: &[i16]) -> Vec<VadOut> {
        let mut out = Vec::new();
        for window in self.chunker.push(&i16_to_f32(frame)) {
            self.detector.accept_waveform(&window);
            self.track(&mut out);
            self.drain(&mut out);
        }
        out
    }

    pub fn flush(&mut self) -> Vec<VadOut> {
        let mut out = Vec::new();
        if let Some(last) = self.chunker.take_padded() {
            self.detector.accept_waveform(&last);
        }
        self.detector.flush();
        self.track(&mut out);
        self.drain(&mut out);
        if self.speaking {
            self.speaking = false;
            out.push(VadOut::End);
        }
        out
    }

    pub fn reset(&mut self) {
        self.detector.reset();
        self.chunker.clear();
        self.speaking = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::ListenConfig;
    use crate::pcm::Resampler;
    use crate::wav::parse_wav;

    #[test]
    fn edges() {
        assert_eq!(edge(false, true), Some(Edge::Start));
        assert_eq!(edge(true, false), Some(Edge::End));
        assert_eq!(edge(true, true), None);
        assert_eq!(edge(false, false), None);
    }

    fn model() -> Option<std::path::PathBuf> {
        let path = ListenConfig::default().vad_path();
        path.exists().then_some(path)
    }

    #[test]
    fn missing_model_is_an_error() {
        assert!(Vad::new(Path::new("/nonexistent/silero.onnx"), 700).is_err());
    }

    #[test]
    fn silence_gives_no_phrase() {
        let Some(path) = model() else {
            eprintln!("skip: no vad model");
            return;
        };
        let mut vad = Vad::new(&path, 700).unwrap();
        let mut out = Vec::new();
        for _ in 0..50 {
            out.extend(vad.push(&[0i16; 320]));
        }
        out.extend(vad.flush());
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn speech_gives_start_end_and_a_phrase() {
        let Some(path) = model() else {
            eprintln!("skip: no vad model");
            return;
        };
        let wav_path = ListenConfig::default().parakeet_dir().join("test_wavs/en.wav");
        let Ok(bytes) = std::fs::read(&wav_path) else {
            eprintln!("skip: no test wav");
            return;
        };
        let wav = parse_wav(&bytes).unwrap();
        let mono = crate::pcm::to_mono(&wav.samples, wav.channels as usize);
        let speech = f32_to_i16(&Resampler::new(wav.rate, SAMPLE_RATE).push(&mono));
        let mut vad = Vad::new(&path, 700).unwrap();
        let mut out = Vec::new();
        let silence = vec![0i16; SAMPLE_RATE as usize];
        for frame in silence.chunks(333).chain(speech.chunks(333)).chain(silence.chunks(333)) {
            out.extend(vad.push(frame));
        }
        out.extend(vad.flush());
        let phrases: Vec<usize> = out.iter().filter_map(|o| if let VadOut::Phrase(p) = o { Some(p.len()) } else { None }).collect();
        assert!(out.first() == Some(&VadOut::Start), "{:?}", out.first());
        assert!(out.contains(&VadOut::End));
        assert!(!phrases.is_empty());
        assert!(phrases.iter().sum::<usize>() > SAMPLE_RATE as usize);
    }
}
