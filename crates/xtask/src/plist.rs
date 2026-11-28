use crate::compile::MAIN_BIN;
use crate::opts::{APP_NAME, BUNDLE_ID};

pub const ICON_FILE: &str = "AppIcon";
pub const MIN_MACOS: &str = "11.0";
pub const MIC_KEY: &str = "NSMicrophoneUsageDescription";
pub const INPUT_KEY: &str = "NSInputMonitoringUsageDescription";
pub const ACCESS_KEY: &str = "NSAccessibilityUsageDescription";
pub const SCREEN_KEY: &str = "NSScreenCaptureUsageDescription";
pub const MIC_TEXT: &str = "Nixa listens to your voice only when you turn on talk or push to talk. Speech is turned into text on this Mac.";
pub const INPUT_TEXT: &str = "Nixa watches for the F9 talk key while another app is in front.";
pub const ACCESS_TEXT: &str = "Nixa reads and uses other apps for your tasks.";
pub const SCREEN_TEXT: &str = "Nixa looks at your screen only when you ask it to.";
pub const NEEDED_KEYS: [&str; 8] = ["CFBundleExecutable", "CFBundleIdentifier", "CFBundleIconFile", "CFBundlePackageType", MIC_KEY, INPUT_KEY, ACCESS_KEY, SCREEN_KEY];

pub enum Value {
    Text(String),
    Yes,
}

fn text_of(value: &str) -> Value {
    Value::Text(value.to_string())
}

pub fn entries(version: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("CFBundleDevelopmentRegion", text_of("en")),
        ("CFBundleDisplayName", text_of(APP_NAME)),
        ("CFBundleExecutable", text_of(MAIN_BIN)),
        ("CFBundleIconFile", text_of(ICON_FILE)),
        ("CFBundleIdentifier", text_of(BUNDLE_ID)),
        ("CFBundleInfoDictionaryVersion", text_of("6.0")),
        ("CFBundleName", text_of(APP_NAME)),
        ("CFBundlePackageType", text_of("APPL")),
        ("CFBundleShortVersionString", text_of(version)),
        ("CFBundleVersion", text_of(version)),
        ("LSApplicationCategoryType", text_of("public.app-category.developer-tools")),
        ("LSMinimumSystemVersion", text_of(MIN_MACOS)),
        ("NSHighResolutionCapable", Value::Yes),
        ("NSSupportsAutomaticGraphicsSwitching", Value::Yes),
        (MIC_KEY, text_of(MIC_TEXT)),
        (INPUT_KEY, text_of(INPUT_TEXT)),
        (ACCESS_KEY, text_of(ACCESS_TEXT)),
        (SCREEN_KEY, text_of(SCREEN_TEXT)),
    ]
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

pub fn text(version: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in entries(version) {
        out.push_str(&format!("\t<key>{}</key>\n", escape(key)));
        match value {
            Value::Text(text) => out.push_str(&format!("\t<string>{}</string>\n", escape(&text))),
            Value::Yes => out.push_str("\t<true/>\n"),
        }
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

pub const PKG_INFO: &str = "APPL????";

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn plist_has_every_needed_key_and_is_valid() {
        let text = text("1.2.3");
        for key in NEEDED_KEYS {
            assert!(text.contains(&format!("<key>{key}</key>")), "{key}");
        }
        assert!(text.contains("<string>dev.nixa.app</string>"));
        assert!(text.contains("<string>nixa-app</string>"));
        assert!(text.contains("<string>Nixa</string>"));
        assert!(text.contains("<key>CFBundleShortVersionString</key>\n\t<string>1.2.3</string>"));
        assert!(text.contains("<key>NSHighResolutionCapable</key>\n\t<true/>"));
        let dir = crate::testkit::temp("plist");
        let path = dir.join("Info.plist");
        std::fs::write(&path, &text).unwrap();
        let out = Command::new("/usr/bin/plutil").arg("-lint").arg(&path).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        let out = Command::new("/usr/bin/plutil").args(["-extract", MIC_KEY, "raw"]).arg(&path).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), MIC_TEXT);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(escape("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert!(text("1 < 2").contains("<string>1 &lt; 2</string>"));
    }
}
