use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};

pub fn panic_line(info: &std::panic::PanicHookInfo<'_>) -> String {
    let what = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into());
    let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
    let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
    format!("panic in thread {thread} at {place}: {what}")
}

pub fn log_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(move |info| {
            crate::util::log("brain", &panic_line(info));
        }));
    });
}

pub fn stop_signal(quit_ms: Option<u64>, asked: Option<std::sync::Arc<tokio::sync::Notify>>) -> impl std::future::Future<Output = String> {
    let own = asked.is_none();
    let term = own.then(|| signal(SignalKind::terminate()).ok()).flatten();
    let hup = own.then(|| signal(SignalKind::hangup()).ok()).flatten();
    async move {
        if let Some(asked) = asked {
            if let Some(ms) = quit_ms {
                tokio::select! {
                    _ = asked.notified() => return "stop asked by the app".into(),
                    _ = tokio::time::sleep(Duration::from_millis(ms)) => return format!("quit after {ms} ms"),
                }
            }
            asked.notified().await;
            return "stop asked by the app".into();
        }
        if let Some(ms) = quit_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            return format!("quit after {ms} ms");
        }
        let (Some(mut term), Some(mut hup)) = (term, hup) else {
            let _ = tokio::signal::ctrl_c().await;
            return "SIGINT".into();
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT".into(),
            _ = term.recv() => "SIGTERM".into(),
            _ = hup.recv() => "SIGHUP".into(),
        }
    }
}
