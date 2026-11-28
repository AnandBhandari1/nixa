use anyhow::bail;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut device = None;
    let mut voice = "Harper".to_string();
    let mut words = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--device" => {
                device = args.get(i + 1).cloned();
                i += 2;
            }
            "--voice" => {
                if let Some(v) = args.get(i + 1) {
                    voice = v.clone();
                }
                i += 2;
            }
            _ => {
                words.push(args[i].clone());
                i += 1;
            }
        }
    }
    let Some(device) = device.filter(|d| !d.trim().is_empty()) else {
        bail!("say needs --device <name>, so nothing plays on the speakers");
    };
    if words.is_empty() {
        bail!("say needs the words to say");
    }
    nixa_voice::set_output_device(Some(device.clone()));
    let said = nixa_voice::speak_wait(&words.join(" "), &voice);
    println!("said on {device} with {voice} in {} ms, interrupted {}", said.ms, said.interrupted);
    Ok(())
}
