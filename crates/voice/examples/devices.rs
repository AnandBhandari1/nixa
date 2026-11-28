use nixa_voice::{ListenConfig, VoiceEvent, list_input_devices, list_output_devices, list_voices};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    println!("Microphones:");
    for d in list_input_devices()? {
        println!("  {}{}", d.name, if d.default { "  (default)" } else { "" });
    }
    println!("Speakers:");
    for d in list_output_devices()? {
        println!("  {}{}", d.name, if d.default { "  (default)" } else { "" });
    }
    let voices = list_voices();
    println!("Voices: {} ({})", voices.len(), voices.iter().take(6).cloned().collect::<Vec<_>>().join(", "));
    let config = ListenConfig::default();
    println!("Models: {}", config.models_dir.display());
    println!("  vad {}", if config.vad_path().exists() { "found" } else { "missing" });
    println!("  speech to text {}", if config.parakeet_dir().exists() { "found" } else { "missing" });
    let secs: u64 = std::env::args()
        .skip_while(|a| a != "--listen")
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if secs == 0 {
        return Ok(());
    }
    println!("Listening for {secs} s");
    let mut loud = 0.0f32;
    nixa_voice::start_listening(config, move |event| match event {
        VoiceEvent::Level { level, .. } => {
            if level > loud {
                loud = level;
                println!("  level {level:.4}");
            }
        }
        other => println!("  {other:?}"),
    })?;
    std::thread::sleep(Duration::from_secs(secs));
    nixa_voice::stop();
    Ok(())
}
