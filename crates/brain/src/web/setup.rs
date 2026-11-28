use crate::app::App;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const ON_KEY: &str = "web.on";
pub const PROVIDER_KEY: &str = "web.provider";
pub const FOLDER_KEY: &str = "web.searxng.folder";
pub const SETTINGS_KEY: &str = "web.searxng.settings";
pub const PORT_KEY: &str = "web.searxng.port";
pub const DEFAULT_FOLDER: &str = ".local/share/searxng/src";
pub const DEFAULT_SETTINGS: &str = ".config/searxng/settings.yml";
pub const DEFAULT_PORT: u16 = 8888;
pub const REFUSED_PORTS: [u16; 4] = [7321, 7330, 7331, 7421];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Searxng,
    Exa,
}

impl Provider {
    pub fn word(self) -> &'static str {
        match self {
            Provider::Searxng => "searxng",
            Provider::Exa => "exa",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Searxng => "local SearXNG",
            Provider::Exa => "Exa",
        }
    }

    pub fn from_word(word: &str) -> Option<Provider> {
        match word.trim().to_ascii_lowercase().as_str() {
            "searxng" => Some(Provider::Searxng),
            "exa" => Some(Provider::Exa),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub on: bool,
    pub provider: Provider,
    pub folder: PathBuf,
    pub settings: PathBuf,
    pub port: u16,
    pub folder_set: bool,
    pub settings_set: bool,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn path_or(text: &str, default: &str, home: &Path) -> PathBuf {
    let text = text.trim();
    if text.is_empty() {
        home.join(default)
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(text)
    }
}

pub fn port_ok(port: i64) -> Option<u16> {
    u16::try_from(port).ok().filter(|p| *p >= 1024 && !REFUSED_PORTS.contains(p))
}

pub fn load_with(conn: &nixa_store::Connection, home: &Path) -> Setup {
    let text = |key: &str| nixa_store::settings::setting_text(conn, key, "").unwrap_or_default();
    let folder = text(FOLDER_KEY);
    let settings = text(SETTINGS_KEY);
    Setup {
        on: nixa_store::settings::setting_bool(conn, ON_KEY, false).unwrap_or(false),
        provider: Provider::from_word(&text(PROVIDER_KEY)).unwrap_or(Provider::Searxng),
        folder: path_or(&folder, DEFAULT_FOLDER, home),
        settings: path_or(&settings, DEFAULT_SETTINGS, home),
        port: port_ok(nixa_store::settings::setting_i64(conn, PORT_KEY, DEFAULT_PORT as i64).unwrap_or(DEFAULT_PORT as i64)).unwrap_or(DEFAULT_PORT),
        folder_set: !folder.trim().is_empty(),
        settings_set: !settings.trim().is_empty(),
    }
}

fn store(app: &App) -> Result<nixa_store::Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("The store did not open: {e:#}"))
}

pub fn load(app: &App) -> Setup {
    match store(app) {
        Ok(conn) => load_with(&conn, &home()),
        Err(_) => load_with_defaults(&home()),
    }
}

pub fn load_with_defaults(home: &Path) -> Setup {
    Setup {
        on: false,
        provider: Provider::Searxng,
        folder: home.join(DEFAULT_FOLDER),
        settings: home.join(DEFAULT_SETTINGS),
        port: DEFAULT_PORT,
        folder_set: false,
        settings_set: false,
    }
}

fn clean_path(value: &Value, key: &str) -> Result<Option<String>, String> {
    let Some(raw) = value.get(key) else { return Ok(None) };
    let text = raw.as_str().ok_or_else(|| format!("{key} must be text."))?.trim().to_string();
    if !text.is_empty() && !(text.starts_with('/') || text.starts_with("~/")) {
        return Err(format!("{key} must be a full path, or empty for the default."));
    }
    if text.contains("..") || text.chars().any(char::is_control) {
        return Err(format!("{key} has a bad path."));
    }
    Ok(Some(text))
}

pub fn change_with(conn: &nixa_store::Connection, value: &Value) -> Result<(), String> {
    let on = match value.get("on") {
        None => None,
        Some(v) => Some(v.as_bool().ok_or("on must be true or false.")?),
    };
    let provider = match value.get("provider") {
        None => None,
        Some(v) => Some(v.as_str().and_then(Provider::from_word).ok_or("provider is searxng or exa.")?),
    };
    let port = match value.get("port") {
        None => None,
        Some(v) => Some(v.as_i64().and_then(port_ok).ok_or("port must be a number from 1024 to 65535 and not a Nixa port.")?),
    };
    let folder = clean_path(value, "folder")?;
    let settings = clean_path(value, "settings")?;
    let save = |done: anyhow::Result<()>| done.map_err(|e| format!("The web setting was not saved: {e:#}"));
    if let Some(on) = on {
        save(nixa_store::settings::set_setting_bool(conn, ON_KEY, on))?;
    }
    if let Some(provider) = provider {
        save(nixa_store::settings::set_setting_text(conn, PROVIDER_KEY, provider.word()))?;
    }
    if let Some(port) = port {
        save(nixa_store::settings::set_setting(conn, PORT_KEY, &json!(port)))?;
    }
    if let Some(folder) = folder {
        save(nixa_store::settings::set_setting_text(conn, FOLDER_KEY, &folder))?;
    }
    if let Some(settings) = settings {
        save(nixa_store::settings::set_setting_text(conn, SETTINGS_KEY, &settings))?;
    }
    Ok(())
}

pub fn change(app: &App, value: &Value) -> Result<(), String> {
    change_with(&store(app)?, value)
}

fn shown(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

pub fn view_of(setup: &Setup, exa_key: bool, installed: bool, running: bool, home: &Path) -> Value {
    json!({
        "on": setup.on,
        "provider": setup.provider.word(),
        "providers": [Provider::Searxng.word(), Provider::Exa.word()],
        "exa_key": exa_key,
        "searxng": {
            "installed": installed,
            "running": running,
            "port": setup.port,
            "folder": shown(&setup.folder, home),
            "settings": shown(&setup.settings, home),
            "folder_set": setup.folder_set,
            "settings_set": setup.settings_set,
        },
    })
}
