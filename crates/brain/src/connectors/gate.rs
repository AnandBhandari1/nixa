use super::manifest::{Action, Permission, Reach};
use regex::Regex;
use std::sync::LazyLock;

static MONEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:pay(?:s|ing|ment|ments|out|outs|roll)?|refund(?:s|ed|ing)?|invoice[sd]?|charge[sd]?|billing|transfer(?:s|red|ring)?|wire|purchase[sd]?|buy|checkout|donat\w*|subscribe[sd]?|salary|salaries)\b|[$€£₹]\s?\d|\b\d[\d,.]*\s?(?:usd|myr|inr|eur|gbp|rm)\b").expect("money")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    None,
    User,
    Trusted,
}

impl By {
    pub fn word(self) -> &'static str {
        match self {
            By::None => "none",
            By::User => "user",
            By::Trusted => "trusted",
        }
    }
}

pub fn money_text(text: &str) -> bool {
    MONEY.is_match(text)
}

pub fn outside_write(action: &Action) -> bool {
    action.permission == Permission::Write && action.reach == Reach::Outside
}
