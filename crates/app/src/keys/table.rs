use gpui::Keystroke;

pub const GLOBAL: &str = "global";
pub const OFF: &str = "none";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub name: String,
}

pub fn entry(key: &str, name: &str) -> Entry {
    Entry {
        key: clean_key(key).unwrap_or_else(|| key.to_string()),
        name: name.to_string(),
    }
}

pub fn is_off(name: &str) -> bool {
    name.is_empty() || name == OFF
}

const DEFAULTS: &[(&str, &str)] = &[
    ("cmd-q", "app.quit"),
    ("cmd-h", "app.hide"),
    ("cmd-alt-h", "app.hide_others"),
    ("cmd-,", "app.settings"),
    ("cmd-k", "app.palette"),
    ("cmd-shift-p", "app.palette"),
    ("cmd-/", "app.help"),
    ("cmd-shift-/", "app.help"),
    ("cmd-=", "app.zoom_in"),
    ("cmd-+", "app.zoom_in"),
    ("cmd--", "app.zoom_out"),
    ("cmd-0", "app.zoom_reset"),
    ("cmd-t", "tab.new"),
    ("cmd-w", "tab.close"),
    ("alt-w", "tab.close"),
    ("ctrl-tab", "tab.next"),
    ("ctrl-shift-tab", "tab.prev"),
    ("cmd-}", "tab.next"),
    ("cmd-{", "tab.prev"),
    ("cmd-shift-]", "tab.next"),
    ("cmd-shift-[", "tab.prev"),
    ("alt-right", "tab.next"),
    ("alt-left", "tab.prev"),
    ("f2", "tab.rename"),
    ("cmd-n", "space.new"),
    ("shift-f2", "space.rename"),
    ("cmd-alt-]", "space.next"),
    ("cmd-alt-[", "space.prev"),
    ("alt-down", "space.next"),
    ("alt-up", "space.prev"),
    ("cmd-]", "agent.next"),
    ("cmd-[", "agent.prev"),
    ("alt-a", "agent.new_menu"),
    ("cmd-d", "pane.split:right"),
    ("cmd-shift-d", "pane.split:down"),
    ("cmd-shift-w", "pane.close"),
    ("cmd-shift-enter", "pane.zoom"),
    ("cmd-alt-left", "pane.go:left"),
    ("cmd-alt-right", "pane.go:right"),
    ("cmd-alt-up", "pane.go:up"),
    ("cmd-alt-down", "pane.go:down"),
    ("cmd-alt-shift-left", "pane.swap:left"),
    ("cmd-alt-shift-right", "pane.swap:right"),
    ("cmd-alt-shift-up", "pane.swap:up"),
    ("cmd-alt-shift-down", "pane.swap:down"),
    ("cmd-b", "layout.toggle:sidebar"),
    ("alt-m", "menu.open"),
    ("alt-t", "menu.tasks"),
    ("alt-p", "menu.project_commands"),
    ("alt-f", "menu.favourites"),
    ("cmd-p", "picker.open"),
    ("f1", "picker.open"),
    ("alt-shift-m", "nixa.screen"),
    ("alt-v", "nixa.listen"),
    ("alt-shift-v", "nixa.toggle"),
];

pub fn defaults() -> Vec<Entry> {
    let mut list: Vec<Entry> = DEFAULTS.iter().map(|(k, n)| entry(k, n)).collect();
    for number in 1..=9 {
        list.push(entry(&format!("cmd-{number}"), &format!("tab.select:{number}")));
    }
    for number in 1..=9 {
        list.push(entry(&format!("ctrl-{number}"), &format!("space.select:{number}")));
    }
    list
}

pub fn clean_key(key: &str) -> Option<String> {
    let parts: Vec<String> = key
        .split_whitespace()
        .map(|k| Keystroke::parse(k).ok().map(|s| s.unparse()))
        .collect::<Option<Vec<String>>>()?;
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub fn merged(defaults: Vec<Entry>, user: &[Entry]) -> Vec<Entry> {
    let mut out = defaults;
    for u in user {
        out.retain(|o| o.key != u.key);
        out.push(u.clone());
    }
    out
}

fn key_word(key: &str) -> String {
    let named = match key {
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "enter" => "↩",
        "tab" => "⇥",
        "backspace" => "⌫",
        "delete" => "⌦",
        "escape" => "⎋",
        "space" => "Space",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page Up",
        "pagedown" => "Page Down",
        _ => "",
    };
    if named.is_empty() {
        key.to_uppercase()
    } else {
        named.to_string()
    }
}

fn shown_stroke(stroke: &str) -> String {
    let Ok(k) = Keystroke::parse(stroke) else {
        return stroke.to_string();
    };
    let mut out = String::new();
    if k.modifiers.function {
        out.push_str("fn ");
    }
    if k.modifiers.control {
        out.push('⌃');
    }
    if k.modifiers.alt {
        out.push('⌥');
    }
    if k.modifiers.shift {
        out.push('⇧');
    }
    if k.modifiers.platform {
        out.push('⌘');
    }
    out.push_str(&key_word(&k.key));
    out
}

pub fn shown(key: &str) -> String {
    key.split_whitespace()
        .map(shown_stroke)
        .collect::<Vec<String>>()
        .join(" ")
}

pub const GROUPS: [&str; 7] = ["App", "Tabs", "Spaces", "Panes", "Agents and Nixa", "Menus", "Turned off"];

pub fn group_of(name: &str) -> &'static str {
    if is_off(name) {
        return GROUPS[6];
    }
    let base = name.split('.').next().unwrap_or("");
    match base {
        "app" => GROUPS[0],
        "tab" => GROUPS[1],
        "space" => GROUPS[2],
        "pane" | "layout" => GROUPS[3],
        "agent" | "nixa" => GROUPS[4],
        _ => GROUPS[5],
    }
}

pub fn groups(entries: &[Entry]) -> Vec<(&'static str, Vec<Entry>)> {
    GROUPS
        .iter()
        .map(|g| {
            let list: Vec<Entry> = entries
                .iter()
                .filter(|e| group_of(&e.name) == *g)
                .cloned()
                .collect();
            (*g, list)
        })
        .filter(|(_, list)| !list.is_empty())
        .collect()
}
