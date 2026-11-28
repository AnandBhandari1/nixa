use nixa_phone::{Deps, Door, DoorNames, sink};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let data = std::env::var_os("NIXA_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support/Nixa Dev"));
    let quit = std::env::var("NIXA_PHONE_QUIT_AFTER_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
    if std::env::var("NIXA_PHONE_SAY").is_ok() {
        let synth = sink::ptts_synth(sink::ptts_bin());
        let got = tokio::task::spawn_blocking(move || synth("Hello from the phone door.", "Toby", &CancellationToken::new())).await?;
        match got {
            Some(pcm) => println!("ptts: {} Hz, {} samples", pcm.rate, pcm.pcm.len() / 2),
            None => println!("ptts: no sound"),
        }
    }
    let mut deps = Deps::new(&data, Arc::new(|| serde_json::json!({"state": "idle"})));
    if let Ok(address) = std::env::var("NIXA_PHONE_ADDRESS") {
        let names = DoorNames { ips: vec![address.clone()], dns: Vec::new() };
        deps.address = Arc::new(move || Some(address.clone()));
        deps.names = Arc::new(move |_| names.clone());
    }
    let (door, mut events) = Door::new(deps)?;
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            eprintln!("event: {event:?}");
        }
    });
    door.start().await;
    let mut info = serde_json::to_value(door.info())?;
    if let Some(map) = info.as_object_mut() {
        let rows = map.get("pair_qr").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        map.insert("pair_qr".to_string(), serde_json::json!(format!("{rows} rows")));
    }
    println!("{}", serde_json::to_string_pretty(&info)?);
    tokio::time::sleep(Duration::from_millis(quit)).await;
    door.stop().await;
    println!("stopped, ports {:?}", door.ports());
    Ok(())
}
