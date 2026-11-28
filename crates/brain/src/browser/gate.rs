use crate::web::safety::PUBLIC_PAGE;
use regex::Regex;
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

pub const ELEMENT_CHARS: usize = 2000;
pub const SAFE_KEYS: [&str; 10] = ["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight", "Tab", "Escape", "PageDown", "PageUp", "Home", "End"];
pub const NEED_REF: &str = "This step needs an element ref from a fresh page snapshot.";
pub const EXACT_REF: &str = "Read the page again and use its exact element ref.";

static PAGE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:Page URL|URL):\s*(https?://\S+|about:blank)").expect("page url"));
static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[ref=(e\d+)\]").expect("ref"));
static REF_SHAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^e\d+$").expect("ref shape"));
static DESTINATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)/url:\s*(\S+)").expect("destination"));
static TAB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- (\d+):(?: \(current\))? \[(.*)\]\((https?://\S+|about:blank)\)").expect("tab"));

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub url: String,
    pub refs: HashMap<String, String>,
    pub stamp: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepArgs {
    pub target: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub index: Option<i64>,
    pub accept: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub url: String,
    pub element: String,
}

pub fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(*c, '\u{200B}'..='\u{200D}' | '\u{FEFF}' | '\u{2060}' | '\u{00AD}'))
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' | '\u{00A0}' => ' ',
            _ => c,
        })
        .collect()
}

fn indent_of(line: &str) -> Option<usize> {
    line.find(|c: char| !c.is_whitespace())
}

pub fn stamp_of(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn page_of(snapshot: &str) -> Page {
    let url = PAGE_URL.captures(snapshot).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
    let lines: Vec<&str> = snapshot.split('\n').collect();
    let mut refs = HashMap::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(found) = REF.captures(line).and_then(|c| c.get(1)) else { continue };
        let indent = indent_of(line).unwrap_or(0);
        let mut parts = vec![line.trim().to_string()];
        for next in &lines[at + 1..] {
            if !indent_of(next).is_some_and(|i| i > indent) {
                break;
            }
            parts.push(next.trim().to_string());
        }
        let joined: String = parts.join(" ").chars().take(ELEMENT_CHARS).collect();
        refs.insert(found.as_str().to_string(), fold(&joined));
    }
    Page { url, refs, stamp: stamp_of(snapshot), snapshot: snapshot.to_string() }
}

pub fn tabs_of(text: &str) -> BTreeMap<i64, Tab> {
    text.split('\n')
        .filter_map(|line| {
            let c = TAB.captures(line)?;
            let index = c.get(1)?.as_str().parse().ok()?;
            Some((index, Tab { title: c.get(2)?.as_str().to_string(), url: c.get(3)?.as_str().to_string() }))
        })
        .collect()
}

pub fn check_tab(tab: &Tab) -> Result<Target, String> {
    let title: String = fold(&tab.title).chars().take(ELEMENT_CHARS).collect();
    Ok(Target { url: tab.url.clone(), element: if title.trim().is_empty() { "tab".into() } else { title } })
}

pub fn check_step(page: &Page, action: &str, args: &StepArgs) -> Result<Target, String> {
    let mut url = match &args.url {
        Some(u) => {
            let parsed = Url::parse(u.trim()).map_err(|_| PUBLIC_PAGE.to_string())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(PUBLIC_PAGE.into());
            }
            parsed.to_string()
        }
        None => page.url.clone(),
    };
    let target = args.target.clone().unwrap_or_default();
    let element = if target.is_empty() { None } else { page.refs.get(&target).cloned() };
    if !target.is_empty() && (!REF_SHAPE.is_match(&target) || element.is_none()) {
        return Err(EXACT_REF.into());
    }
    let destination = element.as_deref().and_then(|e| DESTINATION.captures(e)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
    if let Some(destination) = destination
        && matches!(action, "click" | "hover")
    {
        let joined = Url::parse(&page.url).and_then(|base| base.join(&destination)).map_err(|_| PUBLIC_PAGE.to_string())?;
        url = joined.to_string();
    }
    if matches!(action, "click" | "type" | "fill" | "choose" | "hover") && element.is_none() {
        return Err(NEED_REF.into());
    }
    Ok(Target { url, element: element.unwrap_or_else(|| "current page".into()) })
}

pub fn other_site(target: &str, page: &str) -> bool {
    if !target.starts_with("http") {
        return false;
    }
    if !page.starts_with("http") {
        return true;
    }
    let origin = |u: &str| Url::parse(u).ok().map(|u| u.origin().ascii_serialization());
    origin(target) != origin(page)
}
