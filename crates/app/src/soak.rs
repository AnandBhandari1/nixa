pub const BACKGROUND_ENV: &str = "NIXA_APP_BACKGROUND";
pub const DATA_ENV: &str = "NIXA_DATA_DIR";
pub const SOAK_PREFIXES: [&str; 2] = ["/tmp/nixa-soak-", "/private/tmp/nixa-soak-"];
pub const TITLE: &str = "Nixa";
pub const SOAK_TITLE: &str = "Nixa soak";

pub fn is_soak(data_dir: Option<&str>, background: Option<&str>) -> bool {
    background.is_some_and(|v| v.trim() == "1")
        || data_dir.is_some_and(|d| SOAK_PREFIXES.iter().any(|p| d.starts_with(p)))
}

pub fn now() -> bool {
    is_soak(
        std::env::var(DATA_ENV).ok().as_deref(),
        std::env::var(BACKGROUND_ENV).ok().as_deref(),
    )
}

pub fn title(soak: bool) -> &'static str {
    if soak { SOAK_TITLE } else { TITLE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soak_runs_are_found_by_folder_or_switch() {
        assert!(is_soak(Some("/tmp/nixa-soak-123"), None));
        assert!(is_soak(Some("/private/tmp/nixa-soak-9"), None));
        assert!(is_soak(None, Some("1")));
        assert!(!is_soak(Some("/tmp/other"), Some("0")));
        assert!(!is_soak(None, None));
        assert_eq!(title(true), "Nixa soak");
        assert_eq!(title(false), "Nixa");
    }
}
