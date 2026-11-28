mod aec;
mod devices;
mod keys;
mod listen;
mod mic;
mod models_fetch;
mod own_voice;
mod pcm;
mod scrub;
mod secret;
mod speak;
mod stt;
mod vad;
mod wav;
mod words;

pub use aec::{EchoMic, echo_cancelled, mic_open_during_voice, open_echo_mic, ready_line, use_echo_cancel};
pub use models_fetch::{
    FileState, MODELS_URL, MODELS_URL_ENV, Model, ModelFile, Step, Unpack, check, fetch, missing_models, models,
    models_dir, models_url,
};
pub use devices::{AudioDevice, list_input_devices, list_output_devices, pick_name};
pub use keys::{KeyTap, key_access};
pub use listen::{
    Ignored, Interrupt, ListenConfig, Via, VoiceEvent, hold_end, hold_start, holding,
    is_listening, start_listening, stop,
};
pub use own_voice::{OwnVoice, real_words};
pub use pcm::{
    Chunker, Resampler, SAMPLE_RATE, VAD_WINDOW, f32_to_i16, i16_to_f32, level, to_mono,
};
pub use scrub::scrub_for_speech;
pub use secret::key;
pub use speak::{
    AGENT_VOICES, CYCLE_VOICES, SayResult, SpeechHandle, chunks, is_speaking, list_voices,
    output_device, set_output_device, speak, speak_cancellable, speak_wait, speak_with_fallback, speak_with_parts, stop_speaking, voice_for,
};
pub use stt::{Transcript, parakeet_paths, stt_ready, transcribe};
pub use vad::{Edge, Vad, VadOut, edge};
pub use wav::{Wav, parse_wav, wav_bytes};
pub use words::{
    WAKE_ASK_MS, WAKE_CALL_MS, WAKE_FOLLOW_MS, Words, normalise, names_for, wake_open,
    wake_open_until,
};
