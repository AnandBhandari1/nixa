use crate::util::{read_json, write_json};
use nixa_voice::{Interrupt, ListenConfig, names_for, voice_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const FILE: &str = "voice.json";
pub const FRONT_APPS: [&str; 3] = ["dev.nixa.app", "Nixa", "nixa-app"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Listen {
    #[default]
    Hotkey,
    #[serde(alias = "wake")]
    Always,
}

impl Listen {
    pub fn parse(text: &str) -> Option<Listen> {
        match text.trim().to_lowercase().as_str() {
            "hotkey" | "f9" | "key" | "push" => Some(Listen::Hotkey),
            "always" | "wake" => Some(Listen::Always),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Listen::Hotkey => "hotkey",
            Listen::Always => "always",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceSettings {
    pub mic_on: bool,
    pub voice_on: bool,
    pub listen: Listen,
    pub voice: Option<String>,
    pub mic_device: Option<String>,
    pub speaker_device: Option<String>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings {
            mic_on: false,
            voice_on: false,
            listen: Listen::Hotkey,
            voice: None,
            mic_device: None,
            speaker_device: None,
        }
    }
}

pub fn load(path: &Path) -> VoiceSettings {
    read_json::<Value>(path).map(from_value).unwrap_or_default()
}

pub fn from_value(mut value: Value) -> VoiceSettings {
    if let Some(obj) = value.as_object_mut()
        && let Some(push) = obj.remove("pushTalk")
    {
        let mode = if push.as_bool().unwrap_or(false) { Listen::Hotkey } else { Listen::Always };
        obj.insert("listen".into(), json!(mode.word()));
    }
    serde_json::from_value(value).unwrap_or_default()
}

pub fn load_at_start(path: &Path) -> VoiceSettings {
    VoiceSettings { mic_on: false, ..load(path) }
}

pub fn save(path: &Path, settings: &VoiceSettings) -> std::io::Result<()> {
    write_json(path, settings)
}

pub fn wants_listener(s: &VoiceSettings) -> bool {
    s.mic_on || s.listen == Listen::Hotkey
}

pub fn key_ready(s: &VoiceSettings, models_missing: bool) -> bool {
    s.mic_on || !models_missing
}

pub fn voice_name(s: &VoiceSettings) -> String {
    s.voice.clone().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| voice_for("nixa").to_string())
}

pub fn listen_config(s: &VoiceSettings, assistant: &str, models: &Path, echo_cancel: bool, headphones: bool) -> ListenConfig {
    ListenConfig {
        models_dir: PathBuf::from(models),
        input_device: s.mic_device.clone().filter(|d| !d.trim().is_empty()),
        names: names_for(assistant),
        interrupt: Interrupt::Words,
        hold_key: s.listen == Listen::Hotkey,
        hold_only: s.listen == Listen::Hotkey,
        front_bundles: FRONT_APPS.iter().map(|s| s.to_string()).collect(),
        echo_cancel,
        headphones,
        ..ListenConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn mic_and_voice_start_off_and_the_hotkey_is_the_default() {
        let s = VoiceSettings::default();
        assert!(!s.mic_on && !s.voice_on);
        assert_eq!(s.listen, Listen::Hotkey);
        assert!(wants_listener(&s));
        assert_eq!(voice_name(&s), "Pippa");
    }

    #[test]
    fn the_key_waits_for_the_voice_models_unless_the_mic_is_on() {
        let s = VoiceSettings::default();
        assert!(!key_ready(&s, true));
        assert!(key_ready(&s, false));
        let mic = VoiceSettings { mic_on: true, ..VoiceSettings::default() };
        assert!(key_ready(&mic, true));
    }

    #[test]
    fn old_saved_files_map_to_the_two_choices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let read = |text: &str| {
            std::fs::write(&path, text).unwrap();
            load(&path)
        };
        let fresh = read(r#"{"micOn": true}"#);
        assert!(fresh.mic_on && fresh.listen == Listen::Hotkey);
        assert_eq!(read(r#"{"listen": "wake"}"#).listen, Listen::Always);
        assert_eq!(read(r#"{"listen": "wake", "pushTalk": true}"#).listen, Listen::Hotkey);
        assert_eq!(read(r#"{"listen": "always", "pushTalk": false}"#).listen, Listen::Always);
        assert_eq!(read(r#"{"listen": "always", "pushTalk": true}"#).listen, Listen::Hotkey);
        let always = VoiceSettings { listen: Listen::Always, ..VoiceSettings::default() };
        save(&path, &always).unwrap();
        assert_eq!(load(&path).listen, Listen::Always);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("pushTalk"));
    }

    #[test]
    fn listen_words_parse_old_and_new_names() {
        assert_eq!(Listen::parse("hotkey"), Some(Listen::Hotkey));
        assert_eq!(Listen::parse("F9"), Some(Listen::Hotkey));
        assert_eq!(Listen::parse("push"), Some(Listen::Hotkey));
        assert_eq!(Listen::parse("always"), Some(Listen::Always));
        assert_eq!(Listen::parse("wake"), Some(Listen::Always));
        assert_eq!(Listen::parse("loud"), None);
    }

    #[test]
    fn saves_a_private_file_and_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(load(&path), VoiceSettings::default());
        let s = VoiceSettings { mic_on: true, listen: Listen::Always, voice: Some("Harper".into()), ..VoiceSettings::default() };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"micOn\": true") && text.contains("\"listen\": \"always\""), "{text}");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn listen_config_follows_the_settings() {
        let s = VoiceSettings { listen: Listen::Hotkey, mic_device: Some("BlackHole 2ch".into()), ..VoiceSettings::default() };
        let c = listen_config(&s, "Nova", Path::new("/m"), true, false);
        assert!(c.echo_cancel);
        assert!(c.hold_key && c.hold_only);
        assert_eq!(c.input_device.as_deref(), Some("BlackHole 2ch"));
        assert!(c.names.iter().any(|n| n == "Nova") && c.names.iter().any(|n| n == "nixa"));
        assert!(c.front_bundles.iter().any(|b| b == "dev.nixa.app"));
        assert_eq!(c.models_dir, PathBuf::from("/m"));
        let blank = VoiceSettings { mic_device: Some("  ".into()), listen: Listen::Always, ..VoiceSettings::default() };
        assert_eq!(listen_config(&blank, "Nixa", Path::new("/m"), true, false).input_device, None);
        assert!(!listen_config(&blank, "Nixa", Path::new("/m"), true, false).hold_only);
        assert!(!listen_config(&blank, "Nixa", Path::new("/m"), false, false).echo_cancel);
    }
}
