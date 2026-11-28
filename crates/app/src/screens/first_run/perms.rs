use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Perm {
    Mic,
    InputMonitoring,
    Accessibility,
    ScreenRecording,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    Allowed,
    Denied,
    NotAsked,
    Unknown,
}

pub const ALL: [Perm; 4] = [Perm::Mic, Perm::InputMonitoring, Perm::Accessibility, Perm::ScreenRecording];

pub const SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security";

pub fn title(p: Perm) -> &'static str {
    match p {
        Perm::Mic => "Microphone",
        Perm::InputMonitoring => "Input Monitoring",
        Perm::Accessibility => "Accessibility",
        Perm::ScreenRecording => "Screen Recording",
    }
}

pub fn why(p: Perm) -> &'static str {
    match p {
        Perm::Mic => "So Nixa can hear you when you talk to it.",
        Perm::InputMonitoring => "So the Right Command talk key works while another app is in front.",
        Perm::Accessibility => "So Nixa can click and type for you, only after you say yes to each step.",
        Perm::ScreenRecording => "So Nixa can read the screen and take a screenshot when you ask.",
    }
}

pub fn slug(p: Perm) -> &'static str {
    match p {
        Perm::Mic => "mic",
        Perm::InputMonitoring => "input",
        Perm::Accessibility => "accessibility",
        Perm::ScreenRecording => "screen",
    }
}

pub fn pane(p: Perm) -> &'static str {
    match p {
        Perm::Mic => "Privacy_Microphone",
        Perm::InputMonitoring => "Privacy_ListenEvent",
        Perm::Accessibility => "Privacy_Accessibility",
        Perm::ScreenRecording => "Privacy_ScreenCapture",
    }
}

pub fn settings_url(p: Perm) -> String {
    format!("{SETTINGS_URL}?{}", pane(p))
}

pub fn grant_words(g: Grant) -> &'static str {
    match g {
        Grant::Allowed => "Allowed",
        Grant::Denied => "Not allowed",
        Grant::NotAsked => "Not asked yet",
        Grant::Unknown => "Cannot check",
    }
}

pub fn can_ask(p: Perm, g: Grant) -> bool {
    match g {
        Grant::NotAsked => true,
        Grant::Denied => matches!(p, Perm::InputMonitoring | Perm::Accessibility | Perm::ScreenRecording),
        Grant::Allowed | Grant::Unknown => false,
    }
}

pub fn after_ask(p: Perm) -> &'static str {
    match p {
        Perm::Mic => "",
        Perm::InputMonitoring | Perm::Accessibility | Perm::ScreenRecording => {
            "Turn Nixa on in System Settings. macOS may ask you to quit and open Nixa again."
        }
    }
}

pub fn allowed_count(grants: &[(Perm, Grant)]) -> usize {
    grants.iter().filter(|(_, g)| *g == Grant::Allowed).count()
}

pub fn check_all(check: fn(Perm) -> Grant) -> Vec<(Perm, Grant)> {
    ALL.iter().map(|p| (*p, check(*p))).collect()
}

pub fn check(p: Perm) -> Grant {
    #[cfg(target_os = "macos")]
    {
        super::mac::check(p)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = p;
        Grant::Unknown
    }
}

pub fn ask(p: Perm) {
    #[cfg(target_os = "macos")]
    super::mac::ask(p);
    #[cfg(not(target_os = "macos"))]
    let _ = p;
}

pub fn open_outside(url: &str) {
    if !url.starts_with(SETTINGS_URL) {
        return;
    }
    let url = url.to_string();
    std::thread::spawn(move || {
        let _ = Command::new("/usr/bin/open").arg(url).status();
    });
}
