use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Knowledge,
    Memory,
    Web,
    Connector,
    Mix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    pub route: Route,
    pub personal: bool,
    pub live: bool,
    pub reason: &'static str,
}

static PERSONAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(my|mine|our|ours|about me|i (?:own|have|saved?|live|work|said|told))\b").expect("personal words"));
static CONNECTOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(email|emails|inbox|calendar|appointments?|meetings?)\b").expect("connector words"));
static MAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(email|emails|inbox)\b").expect("mail words"));
static LIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(stock|share price|quote|news|headlines|weather|forecast|price|prices|latest|current|today|version|versions|exchange rate)\b")
        .expect("live words")
});
static ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(send|post|buy|order|sell|pay|deploy|publish|push|delete|set|change|write|implement|fix|build|add|remind)\b").expect("action words")
});
static FAST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:what(?:'s| is| are)|show me|tell me|look up|check|find|where is)\b.*\b(?:stock|share price|news|headlines|weather|forecast|price|latest version|email|inbox)\b|(?:stock price|weather|news|headlines)\b)")
        .expect("fast words")
});
static REMIND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^remind me (.+)$").expect("remind words"));
static WHEN_FIRST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(.+?) to (.+)$").expect("when first"));
static WHEN_LAST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^to (.+?) ((?:tomorrow |today )?at .+|in (?:\d+|an?) (?:minute|hour|day)s?)$").expect("when last")
});

pub fn names_in(question: &str, names: &[String]) -> bool {
    let words = question.to_lowercase();
    names.iter().map(|n| n.trim().to_lowercase()).filter(|n| n.chars().count() >= 2).any(|name| {
        words.match_indices(&name).any(|(at, _)| {
            let before = words[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
            let after = words[at + name.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
            before && after
        })
    })
}

pub fn route(question: &str, names: &[String]) -> Routing {
    let personal = PERSONAL.is_match(question) || names_in(question, names);
    let connector = CONNECTOR.is_match(question);
    let live = LIVE.is_match(question);
    let (route, reason) = if connector {
        (Route::Connector, "Read local memory, then the user's own email or calendar connector.")
    } else if personal && live {
        (Route::Mix, "Read memory first; search only a separate public question.")
    } else if personal {
        (Route::Memory, "Search local memory before local knowledge.")
    } else if live {
        (Route::Web, "This fact needs a live source.")
    } else {
        (Route::Knowledge, "Search local knowledge.")
    };
    Routing { route, personal: personal || connector, live: live || connector, reason }
}

pub const ROUTE_PICK: &str = "route";
pub const ROUTE_TEXT: &str = "Where should Nixa look to answer this question? Pick one.";
pub const ROUTE_CHOICES: [(&str, &str); 4] = [
    ("knowledge", "general facts Nixa's local knowledge can answer"),
    ("memory", "about the user, their own saved facts, notes or work"),
    ("web", "needs a live source such as news, prices, weather or latest versions"),
    ("mix", "about the user and also needs a live public fact"),
];

pub fn route_of_word(word: &str) -> Option<Route> {
    match word {
        "knowledge" => Some(Route::Knowledge),
        "memory" => Some(Route::Memory),
        "web" => Some(Route::Web),
        "mix" => Some(Route::Mix),
        _ => None,
    }
}

pub fn with_model(rule: Routing, picked: Route) -> Routing {
    if rule.route == Route::Connector || (rule.personal && picked == Route::Web) {
        return rule;
    }
    let personal = rule.personal || matches!(picked, Route::Memory | Route::Mix);
    let live = rule.live || matches!(picked, Route::Web | Route::Mix);
    Routing { route: picked, personal, live, reason: "The decision model picked the source." }
}

pub fn is_mail(question: &str) -> bool {
    MAIL.is_match(question)
}

pub fn strip_name(question: &str, name: &str) -> String {
    let q = question.trim();
    let name = name.trim();
    if name.is_empty() || q.len() < name.len() || !q.is_char_boundary(name.len()) || !q[..name.len()].eq_ignore_ascii_case(name) {
        return q.to_string();
    }
    let rest = &q[name.len()..];
    let cut = rest.trim_start_matches([',', ' ']);
    if cut.len() == rest.len() && !rest.is_empty() {
        return q.to_string();
    }
    cut.to_string()
}

pub fn is_fast(question: &str, name: &str) -> bool {
    let q = strip_name(question, name);
    !ACTION.is_match(&q) && FAST.is_match(&q)
}

pub fn reminder(question: &str, name: &str) -> Option<(String, String)> {
    let q = strip_name(question, name);
    let rest = REMIND.captures(&q)?.get(1)?.as_str().trim().to_string();
    if let Some(c) = WHEN_LAST.captures(&rest) {
        return Some((c[2].trim().to_string(), c[1].trim().to_string()));
    }
    if let Some(c) = WHEN_FIRST.captures(&rest) {
        return Some((c[1].trim().to_string(), c[2].trim().to_string()));
    }
    Some((rest, "Personal reminder".into()))
}
