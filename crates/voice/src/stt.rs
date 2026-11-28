use crate::pcm::{SAMPLE_RATE, i16_to_f32};
use anyhow::{Result, bail};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub ms: u64,
}

pub struct ParakeetPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

pub fn parakeet_paths(dir: &Path) -> ParakeetPaths {
    ParakeetPaths {
        encoder: dir.join("encoder.int8.onnx"),
        decoder: dir.join("decoder.int8.onnx"),
        joiner: dir.join("joiner.int8.onnx"),
        tokens: dir.join("tokens.txt"),
    }
}

type Loaded = (PathBuf, Arc<OfflineRecognizer>);

static RECOGNIZER: Mutex<Option<Loaded>> = Mutex::new(None);

fn text(p: &Path) -> Option<String> {
    Some(p.to_string_lossy().into_owned())
}

fn load(dir: &Path) -> Result<OfflineRecognizer> {
    let paths = parakeet_paths(dir);
    let missing: Vec<String> = [&paths.encoder, &paths.decoder, &paths.joiner, &paths.tokens]
        .iter()
        .filter(|p| !p.exists())
        .map(|p| p.display().to_string())
        .collect();
    if !missing.is_empty() {
        bail!("parakeet files missing: {}", missing.join(", "));
    }
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: text(&paths.encoder),
        decoder: text(&paths.decoder),
        joiner: text(&paths.joiner),
    };
    config.model_config.tokens = text(&paths.tokens);
    config.model_config.num_threads = 4;
    config.model_config.provider = Some("cpu".into());
    config.model_config.model_type = Some("nemo_transducer".into());
    config.decoding_method = Some("greedy_search".into());
    let Some(recognizer) = OfflineRecognizer::create(&config) else {
        bail!("could not load parakeet from {}", dir.display());
    };
    let warm = recognizer.create_stream();
    warm.accept_waveform(SAMPLE_RATE as i32, &vec![0.0f32; SAMPLE_RATE as usize]);
    recognizer.decode(&warm);
    Ok(recognizer)
}

fn recognizer(dir: &Path) -> Result<Arc<OfflineRecognizer>> {
    let mut slot = RECOGNIZER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, rec)) = slot.as_ref()
        && at == dir
    {
        return Ok(rec.clone());
    }
    let rec = Arc::new(load(dir)?);
    *slot = Some((dir.to_path_buf(), rec.clone()));
    Ok(rec)
}

pub fn stt_ready(dir: &Path) -> Result<u64> {
    let started = Instant::now();
    recognizer(dir)?;
    Ok(started.elapsed().as_millis() as u64)
}

pub fn transcribe(dir: &Path, pcm: &[i16]) -> Result<Transcript> {
    if pcm.is_empty() {
        return Ok(Transcript { text: String::new(), ms: 0 });
    }
    let rec = recognizer(dir)?;
    let started = Instant::now();
    let stream = rec.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, &i16_to_f32(pcm));
    rec.decode(&stream);
    let text = stream.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default();
    Ok(Transcript { text, ms: started.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::ListenConfig;
    use crate::pcm::{Resampler, f32_to_i16, to_mono};
    use crate::wav::parse_wav;

    #[test]
    fn missing_files_are_named() {
        let err = stt_ready(Path::new("/nonexistent/parakeet")).unwrap_err().to_string();
        assert!(err.contains("encoder.int8.onnx"), "{err}");
    }

    #[test]
    fn empty_audio_is_empty_text() {
        let t = transcribe(Path::new("/nonexistent"), &[]).unwrap();
        assert_eq!(t.text, "");
    }

    #[test]
    #[ignore]
    fn transcribes_the_english_test_wav() {
        let config = ListenConfig::default();
        let bytes = std::fs::read(config.parakeet_dir().join("test_wavs/en.wav")).unwrap();
        let wav = parse_wav(&bytes).unwrap();
        let mono = to_mono(&wav.samples, wav.channels as usize);
        let pcm = f32_to_i16(&Resampler::new(wav.rate, SAMPLE_RATE).push(&mono));
        let t = transcribe(&config.parakeet_dir(), &pcm).unwrap();
        eprintln!("heard: {} ({} ms)", t.text, t.ms);
        assert!(t.text.split_whitespace().count() >= 3, "{}", t.text);
    }
}
