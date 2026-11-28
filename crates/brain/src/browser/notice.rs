use crate::app::App;
use reqwest::Url;

pub const COMPANIES: [(&str, &str); 14] = [
    ("codex", "OpenAI"),
    ("agy", "Google"),
    ("kimi", "Moonshot AI"),
    ("cursor", "Cursor"),
    ("claude", "Anthropic"),
    ("openai", "OpenAI"),
    ("anthropic", "Anthropic"),
    ("google", "Google"),
    ("openrouter", "OpenRouter"),
    ("moonshot", "Moonshot AI"),
    ("deepseek", "DeepSeek"),
    ("groq", "Groq"),
    ("xai", "xAI"),
    ("opencode", "OpenCode"),
];

pub const DOMAINS: [(&str, &str); 11] = [
    ("openai.com", "OpenAI"),
    ("anthropic.com", "Anthropic"),
    ("googleapis.com", "Google"),
    ("openrouter.ai", "OpenRouter"),
    ("moonshot.ai", "Moonshot AI"),
    ("moonshot.cn", "Moonshot AI"),
    ("deepseek.com", "DeepSeek"),
    ("groq.com", "Groq"),
    ("x.ai", "xAI"),
    ("azure.com", "Microsoft Azure"),
    ("amazonaws.com", "Amazon Web Services"),
];

fn company(word: &str) -> Option<&'static str> {
    COMPANIES.iter().find(|(k, _)| *k == word).map(|(_, c)| *c)
}

pub fn company_of(kind: &str, model: &str, base_url: Option<&str>) -> String {
    if let Some(base) = base_url.map(str::trim).filter(|b| !b.is_empty()) {
        let host = Url::parse(base).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        if host.is_empty() {
            return "an unknown brain service".into();
        }
        if crate::brains::is_local_url(base) {
            return format!("the local brain on {host}");
        }
        let domain = host.to_lowercase();
        let domain = domain.trim_end_matches('.');
        for (suffix, name) in DOMAINS {
            if domain == suffix || domain.ends_with(&format!(".{suffix}")) {
                return format!("{name} ({host})");
            }
        }
        return format!("the brain service at {host}");
    }
    if kind == "opencode" {
        let provider = model.split_once('/').map(|(p, _)| p).unwrap_or("");
        return if provider.is_empty() { "OpenCode's configured brain provider".into() } else { format!("{} (via OpenCode)", company(provider).unwrap_or(provider)) };
    }
    company(kind).map(str::to_string).unwrap_or_else(|| kind.to_string())
}

pub fn current(app: &App) -> String {
    if let Some(kind) = crate::cli_brains::chosen(app) {
        return company_of(kind.word(), "", None);
    }
    match crate::agent::current_brain(app) {
        Some(brain) => company_of("", &brain.model, Some(&brain.base_url)),
        None => String::new(),
    }
}

pub fn session_line(company: &str) -> String {
    format!("Your picked Chrome tab text will be sent to {company}.")
}
