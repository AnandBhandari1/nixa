use gpui::{Hsla, rgb};
use nixa_ui::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Blue,
    Yellow,
    Cyan,
    Green,
    Magenta,
    Red,
    Muted,
    Grey,
    Accent,
    Plain,
    Secret,
}

const FOLDERS: &[(&str, Tone)] = &[
    ("src", Tone::Blue),
    ("lib", Tone::Blue),
    ("app", Tone::Blue),
    ("apps", Tone::Blue),
    ("packages", Tone::Blue),
    ("components", Tone::Blue),
    ("modules", Tone::Blue),
    ("crates", Tone::Blue),
    ("test", Tone::Green),
    ("tests", Tone::Green),
    ("spec", Tone::Green),
    ("specs", Tone::Green),
    ("__tests__", Tone::Green),
    ("docs", Tone::Cyan),
    ("doc", Tone::Cyan),
    ("documentation", Tone::Cyan),
    ("assets", Tone::Magenta),
    ("public", Tone::Magenta),
    ("static", Tone::Magenta),
    ("images", Tone::Magenta),
    ("img", Tone::Magenta),
    ("media", Tone::Magenta),
    ("scripts", Tone::Yellow),
    ("script", Tone::Yellow),
    ("bin", Tone::Yellow),
    ("config", Tone::Yellow),
    ("configs", Tone::Yellow),
    ("node_modules", Tone::Grey),
    ("dist", Tone::Grey),
    ("build", Tone::Grey),
    ("out", Tone::Grey),
    ("target", Tone::Grey),
    ("coverage", Tone::Grey),
    ("vendor", Tone::Grey),
];

const EXTENSIONS: &[(Tone, &[&str])] = &[
    (
        Tone::Blue,
        &[
            "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "astro",
        ],
    ),
    (
        Tone::Yellow,
        &[
            "py",
            "pyi",
            "ipynb",
            "json",
            "json5",
            "jsonc",
            "yaml",
            "yml",
            "toml",
            "ini",
            "cfg",
            "conf",
            "properties",
            "plist",
            "xml",
            "gradle",
            "sql",
            "sqlite",
            "sqlite3",
        ],
    ),
    (
        Tone::Cyan,
        &[
            "rs", "go", "c", "h", "cc", "cpp", "cxx", "hpp", "hxx", "cs", "java", "kt", "kts",
            "swift", "zig", "dart", "scala", "ex", "exs", "erl", "clj", "cljs", "fs", "fsx", "lua",
            "r", "graphql", "gql", "wasm", "html", "htm", "xhtml",
        ],
    ),
    (
        Tone::Green,
        &["sh", "bash", "zsh", "fish", "ps1", "bat", "cmd"],
    ),
    (
        Tone::Magenta,
        &[
            "css", "scss", "sass", "less", "styl", "png", "jpg", "jpeg", "gif", "webp", "avif",
            "svg", "ico", "bmp", "tiff", "heic", "psd", "pdf",
        ],
    ),
    (Tone::Muted, &["md", "mdx", "rst", "txt", "adoc", "tex"]),
    (
        Tone::Red,
        &[
            "mp3", "wav", "flac", "m4a", "ogg", "mp4", "mov", "webm", "mkv", "avi",
        ],
    ),
];

const NAMES: &[(&str, Tone)] = &[
    ("dockerfile", Tone::Cyan),
    ("makefile", Tone::Yellow),
    ("cmakelists.txt", Tone::Yellow),
    ("readme", Tone::Muted),
    ("license", Tone::Muted),
    ("authors", Tone::Muted),
    ("changelog", Tone::Muted),
];

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "gemfile.lock",
    "poetry.lock",
    "composer.lock",
];

const ARCHIVES: &[&str] = &[
    "zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "dmg", "iso", "deb", "rpm", "map",
];
const SECRET_EXTENSIONS: &[&str] = &[
    "pem", "key", "p12", "pfx", "crt", "cer", "der", "asc", "gpg",
];
const SECRET_NAMES: &[&str] = &["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"];

pub fn extension(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.rfind('.') {
        Some(at) if at > 0 && at + 1 < lower.len() => lower[at + 1..].to_string(),
        _ => String::new(),
    }
}

fn is_env(lower: &str) -> bool {
    lower == ".env" || lower.starts_with(".env.") || (lower.ends_with(".env") && lower.len() > 4)
}

fn is_secret_word(lower: &str) -> bool {
    ["secret", "secrets", "credential", "credentials"]
        .iter()
        .any(|w| lower == *w || lower.starts_with(&format!("{w}.")))
}

pub fn sensitive(name: &str) -> bool {
    let lower = name.to_lowercase();
    is_env(&lower)
        || is_secret_word(&lower)
        || SECRET_NAMES.contains(&lower.as_str())
        || SECRET_EXTENSIONS.contains(&extension(name).as_str())
}

pub fn tone(name: &str, dir: bool) -> Tone {
    let lower = name.to_lowercase();
    if dir {
        if name.starts_with('.') {
            return Tone::Grey;
        }
        return FOLDERS
            .iter()
            .find(|(n, _)| *n == lower)
            .map_or(Tone::Accent, |(_, t)| *t);
    }
    if sensitive(name) {
        return Tone::Secret;
    }
    let ext = extension(name);
    if name.starts_with('.')
        || LOCKFILES.contains(&lower.as_str())
        || ARCHIVES.contains(&ext.as_str())
    {
        return Tone::Grey;
    }
    if let Some((_, t)) = NAMES.iter().find(|(n, _)| *n == lower) {
        return *t;
    }
    EXTENSIONS
        .iter()
        .find(|(_, list)| list.contains(&ext.as_str()))
        .map_or(Tone::Plain, |(t, _)| *t)
}

pub fn color(t: &Theme, tone: Tone) -> Hsla {
    match tone {
        Tone::Blue => t.blue,
        Tone::Yellow => t.yellow,
        Tone::Cyan => t.cyan,
        Tone::Green => t.green,
        Tone::Magenta => t.magenta,
        Tone::Red => t.red,
        Tone::Muted => t.muted,
        Tone::Grey => t.dim,
        Tone::Accent => t.accent,
        Tone::Plain => t.foreground,
        Tone::Secret => rgb(0xb23a3a).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_by_role() {
        assert_eq!(tone("src", true), Tone::Blue);
        assert_eq!(tone("Tests", true), Tone::Green);
        assert_eq!(tone("node_modules", true), Tone::Grey);
        assert_eq!(tone(".github", true), Tone::Grey);
        assert_eq!(tone("anything", true), Tone::Accent);
    }

    #[test]
    fn files_by_type() {
        assert_eq!(tone("main.rs", false), Tone::Cyan);
        assert_eq!(tone("app.tsx", false), Tone::Blue);
        assert_eq!(tone("Cargo.toml", false), Tone::Yellow);
        assert_eq!(tone("Cargo.lock", false), Tone::Grey);
        assert_eq!(tone("README", false), Tone::Muted);
        assert_eq!(tone("notes.md", false), Tone::Muted);
        assert_eq!(tone("run.sh", false), Tone::Green);
        assert_eq!(tone("song.mp3", false), Tone::Red);
        assert_eq!(tone(".gitignore", false), Tone::Grey);
        assert_eq!(tone("plain", false), Tone::Plain);
    }

    #[test]
    fn secrets_are_red() {
        for name in [
            ".env",
            ".env.local",
            "prod.env",
            "id_rsa",
            "server.pem",
            "secrets.json",
            "Credentials",
        ] {
            assert!(sensitive(name), "{name}");
            assert_eq!(tone(name, false), Tone::Secret, "{name}");
        }
        assert!(!sensitive("environment.rs"));
        assert!(!sensitive("secretary.txt"));
        assert!(!sensitive(".envrc"));
    }

    #[test]
    fn extension_needs_a_name_before_the_dot() {
        assert_eq!(extension(".bashrc"), "");
        assert_eq!(extension("a."), "");
        assert_eq!(extension("A.RS"), "rs");
    }
}
