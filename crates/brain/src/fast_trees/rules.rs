use super::phrase::{Hit, PhraseIndex, clean_phrase};
use super::tree::{MAX_DEPTH, Tree, chart_of, compile_tree};
use crate::risk::{hard_stop_text, risky_text};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

pub const ROUTING: &str = "brain-routing";
pub const MAX_KEYWORDS: usize = 512;
pub const MAX_ROWS: usize = 2048;
pub const MAX_ALIASES: usize = 64;
pub const MAX_ALIAS_CHARS: usize = 120;
pub const MAX_ANSWER_RULES: usize = 200;
pub const FACTS: [&str; 5] = ["stop", "hold", "risk", "talk", "review"];
pub const ROUTE_ACTIONS: [&str; 6] = ["stop", "hold", "approval", "plan", "review", "build"];

const DEFAULTS: &str = include_str!("defaults.json");
const MODES: [&str; 3] = ["plan", "build", "review"];
const TIERS: [&str; 3] = ["cheap", "standard", "premium"];
const PLACES: [&str; 4] = ["risk", "mode", "gate", "hint"];
const GATES: [&str; 12] = ["plan-topic", "plan-write", "act", "review-now", "explain", "debate", "yourself", "quick", "later", "stop", "hold", "go-on"];
const TALK: [&str; 8] = ["plan-topic", "plan-write", "explain", "debate", "yourself", "quick", "later", "mode-plan"];
const LEADS: [&str; 13] = ["ok", "okay", "so", "please", "hey", "and", "then", "lets", "let us", "i want to", "can we", "we need to", "nixa"];
const BOUNDS: [&str; 6] = ["but", "then", "just", "instead", "now", "and"];
const LESS: [(&str, &str); 4] = [("best-model", "cheap"), ("full", "quick-check"), ("urgent", "later"), ("everywhere", "this-repo")];

static DENIED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^| )(?:dont|do not|never|without|no)(?: |$)").unwrap());
static ASKING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:(?:can|could|would|should|why|what|how|when|where|is|are)\b|(?:do|does) (?:you|we|i|they)\b)").unwrap());
static KEYWORD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w-]+$").unwrap());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyword {
    pub id: String,
    pub word: String,
    pub aliases: Vec<String>,
    pub effect: String,
    #[serde(rename = "match")]
    pub place: String,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRow {
    pub kind: String,
    pub capability: String,
    pub mode: String,
    pub tier: String,
    pub fallback: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerRule {
    pub id: i64,
    pub project: String,
    pub ask: String,
    pub answer: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub version: u32,
    pub max_depth: usize,
    pub keywords: Vec<Keyword>,
    pub decisions: Vec<DecisionRow>,
    pub trees: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_rules: Option<Vec<AnswerRule>>,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub kind: Option<String>,
    pub capability: Option<String>,
    pub held: bool,
    pub card_open: bool,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchView {
    pub id: String,
    pub phrase: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FastDecision {
    pub kind: String,
    pub capability: String,
    pub mode: String,
    pub tier: String,
    pub fallback: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    pub keywords: Vec<String>,
    pub matches: Vec<MatchView>,
    pub requires_approval: bool,
    pub grants_approval: bool,
    pub action: String,
    pub path: Vec<String>,
    pub comparisons: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeDepth {
    pub name: String,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfCheck {
    pub ok: bool,
    pub problems: Vec<String>,
    pub keywords: usize,
    pub phrases: usize,
    pub table: usize,
    pub trees: Vec<TreeDepth>,
    pub trie_states: usize,
}

pub struct Compiled {
    keywords: Vec<Keyword>,
    index: PhraseIndex,
    max_words: usize,
    phrases: usize,
    leads: Vec<Vec<String>>,
    table: HashMap<(String, String), DecisionRow>,
    trees: BTreeMap<String, Tree>,
}

static SEED: LazyLock<Rules> = LazyLock::new(|| serde_json::from_str(DEFAULTS).expect("the built-in fast decision rules are valid"));

pub fn seed() -> Rules {
    SEED.clone()
}

pub fn seed_value() -> Value {
    serde_json::to_value(&*SEED).unwrap_or(Value::Null)
}

pub fn rules_of(value: &Value) -> Result<Rules, String> {
    if !value.is_object() {
        return Err("Invalid fast-decisions.json: root must be a JSON object.".into());
    }
    serde_json::from_value(value.clone()).map_err(|e| format!("Invalid fast-decisions.json: version, keywords, decisions and trees are required ({e})."))
}

fn fixed() -> Vec<&'static Keyword> {
    SEED.keywords.iter().filter(|k| k.place == "risk").collect()
}

fn placement(effect: &str) -> &'static str {
    if effect == "risk" {
        "risk"
    } else if effect.starts_with("mode-") {
        "mode"
    } else if GATES.contains(&effect) {
        "gate"
    } else {
        "hint"
    }
}

fn priority(row: &Keyword) -> u32 {
    match row.effect.as_str() {
        "stop" => 70,
        "hold" => 60,
        _ if row.place == "risk" => 50,
        e if TALK.contains(&e) => 40,
        "review-now" | "mode-review" => 30,
        "act" | "go-on" | "mode-build" => 20,
        _ => 10,
    }
}

fn sorted(list: &[String]) -> Vec<String> {
    let mut list = list.to_vec();
    list.sort();
    list
}

fn check_keywords(rules: &Rules) -> Result<usize, String> {
    let effects: BTreeSet<&str> = SEED.keywords.iter().map(|k| k.effect.as_str()).collect();
    let risks = fixed();
    let risk_phrases: Vec<String> = risks.iter().flat_map(|r| r.aliases.iter().map(|a| clean_phrase(a))).collect();
    let mut ids = BTreeSet::new();
    let mut aliases: HashMap<String, String> = HashMap::new();
    for row in &rules.keywords {
        let good = KEYWORD_ID.is_match(&row.id)
            && !ids.contains(&row.id)
            && effects.contains(row.effect.as_str())
            && PLACES.contains(&row.place.as_str())
            && !row.aliases.is_empty()
            && row.aliases.len() <= MAX_ALIASES;
        if !good {
            return Err(format!("Invalid or duplicate keyword {}.", row.id));
        }
        ids.insert(row.id.clone());
        let place = placement(&row.effect);
        if row.place != place {
            return Err(format!("Keyword {} needs {place} placement; authority cannot change with its position.", row.id));
        }
        for alias in &row.aliases {
            let text = clean_phrase(alias);
            if alias.chars().count() > MAX_ALIAS_CHARS || text.is_empty() {
                return Err(format!("Empty or oversized alias in {}.", row.id));
            }
            if let Some(other) = aliases.get(&text)
                && other != &row.id
            {
                return Err(format!("Unreachable alias \"{alias}\": both {other} and {}.", row.id));
            }
            aliases.insert(text.clone(), row.id.clone());
            let padded = format!(" {text} ");
            if row.place != "risk" && risk_phrases.iter().any(|p| padded.contains(&format!(" {p} "))) {
                return Err(format!("Risk phrase cannot be reassigned to {}: {alias}.", row.id));
            }
        }
    }
    for risk in risks {
        let found = rules.keywords.iter().find(|k| k.id == risk.id);
        let same = found.is_some_and(|f| f.word == risk.word && f.effect == risk.effect && f.place == risk.place && f.on && sorted(&f.aliases) == sorted(&risk.aliases));
        if !same {
            return Err(format!("Risk keyword {} is fixed and cannot be changed or disabled.", risk.id));
        }
    }
    Ok(aliases.len())
}

fn check_table(rules: &Rules) -> Result<HashMap<(String, String), DecisionRow>, String> {
    let mut table = HashMap::new();
    for row in &rules.decisions {
        let good = !row.kind.is_empty() && !row.capability.is_empty() && MODES.contains(&row.mode.as_str()) && TIERS.contains(&row.tier.as_str()) && row.fallback == "secondary";
        if !good {
            return Err("Invalid decision table row.".into());
        }
        let key = (row.kind.clone(), row.capability.clone());
        if table.contains_key(&key) {
            return Err(format!("Duplicate decision table key {}/{}.", row.kind, row.capability));
        }
        table.insert(key, row.clone());
    }
    if !table.contains_key(&("general".to_string(), "*".to_string())) {
        return Err("Decision table needs general/* as its safe default.".into());
    }
    Ok(table)
}

pub fn required_action(facts: &BTreeMap<String, bool>) -> Option<&'static str> {
    let on = |k: &str| facts.get(k).copied().unwrap_or(false);
    if on("stop") {
        Some("stop")
    } else if on("hold") {
        Some("hold")
    } else if on("risk") {
        Some("approval")
    } else if on("talk") {
        Some("plan")
    } else if on("review") {
        Some("review")
    } else {
        None
    }
}

fn facts_of(bits: u32) -> BTreeMap<String, bool> {
    FACTS.iter().enumerate().map(|(i, k)| (k.to_string(), bits & (1 << i) != 0)).collect()
}

fn check_routing(tree: &Tree) -> Result<(), String> {
    if let Some(bad) = tree.checks().into_iter().find(|k| !FACTS.contains(k)) {
        return Err(format!("Unknown routing condition {bad}."));
    }
    if let Some(bad) = tree.actions().into_iter().find(|k| !ROUTE_ACTIONS.contains(k)) {
        return Err(format!("Unknown routing action {bad}."));
    }
    for bits in 0..(1u32 << FACTS.len()) {
        let facts = facts_of(bits);
        let action = tree.run(&facts)?.action;
        if let Some(required) = required_action(&facts)
            && action != required
        {
            return Err(format!("Brain routing must keep {required}; its edited path reaches {action}."));
        }
    }
    Ok(())
}

fn check_answer_rules(rules: &Rules) -> Result<(), String> {
    let Some(list) = &rules.answer_rules else { return Ok(()) };
    if list.len() > MAX_ANSWER_RULES {
        return Err("Invalid answer rules list.".into());
    }
    let mut seen = BTreeSet::new();
    for row in list {
        let good = row.id >= 1
            && !seen.contains(&row.id)
            && !row.project.is_empty()
            && !row.ask.trim().is_empty()
            && !row.answer.trim().is_empty()
            && ["trial", "offer", "live", "off"].contains(&row.status.as_str())
            && !hard_stop_text(&row.ask)
            && !hard_stop_text(&row.answer);
        if !good {
            return Err(format!("Invalid or unsafe answer rule {}.", row.id));
        }
        seen.insert(row.id);
    }
    Ok(())
}

pub fn compile(rules: &Rules) -> Result<Compiled, String> {
    if rules.version != 1 {
        return Err("Invalid fast-decisions.json: version must be 1.".into());
    }
    if !(1..=MAX_DEPTH).contains(&rules.max_depth) {
        return Err(format!("maxDepth must be 1 to {MAX_DEPTH}."));
    }
    if rules.keywords.len() > MAX_KEYWORDS || rules.decisions.len() > MAX_ROWS {
        return Err("Decision rules exceed the size limit.".into());
    }
    let phrases = check_keywords(rules)?;
    let table = check_table(rules)?;
    let mut trees = BTreeMap::new();
    for (name, raw) in &rules.trees {
        let tree = chart_of(raw).and_then(|chart| compile_tree(&chart, rules.max_depth)).map_err(|e| format!("Tree {name}: {e}"))?;
        trees.insert(name.clone(), tree);
    }
    let routing = trees.get(ROUTING).ok_or("Missing brain-routing tree.")?;
    check_routing(routing)?;
    check_answer_rules(rules)?;
    let list: Vec<(String, usize)> = rules
        .keywords
        .iter()
        .enumerate()
        .filter(|(_, k)| k.on)
        .flat_map(|(i, k)| k.aliases.iter().map(move |a| (a.clone(), i)))
        .collect();
    let index = PhraseIndex::new(&list)?;
    let max_words = rules.keywords.iter().flat_map(|k| k.aliases.iter()).map(|a| clean_phrase(a).split(' ').count()).max().unwrap_or(1);
    let leads = LEADS.iter().map(|l| clean_phrase(l).split(' ').map(str::to_string).collect()).collect();
    let compiled = Compiled { keywords: rules.keywords.clone(), index, max_words, phrases, leads, table, trees };
    let check = compiled.self_check();
    if !check.ok {
        return Err(check.problems.join("\n"));
    }
    Ok(compiled)
}

fn denied_before(words: &[String], at: usize) -> bool {
    let before = &words[at.saturating_sub(8)..at];
    let from = before.iter().rposition(|w| BOUNDS.contains(&w.as_str())).map(|i| i + 1).unwrap_or(0);
    DENIED.is_match(&before[from..].join(" "))
}

impl Compiled {
    fn keyword(&self, hit: &Hit) -> &Keyword {
        &self.keywords[hit.item]
    }

    pub fn find(&self, text: &str, held: bool, card_open: bool) -> Vec<Hit> {
        let (words, hits) = self.index.scan(text);
        let mut starts = BTreeSet::from([0usize]);
        let mut offset = 0;
        while let Some(lead) = self.leads.iter().find(|lead| lead.iter().enumerate().all(|(i, w)| words.get(offset + i) == Some(w))) {
            offset += lead.len();
            starts.insert(offset);
        }
        let question = text.contains('?') || ASKING.is_match(&words.join(" "));
        let eligible: Vec<Hit> = hits
            .into_iter()
            .filter(|hit| {
                let row = self.keyword(hit);
                if denied_before(&words, hit.start) {
                    return false;
                }
                match row.place.as_str() {
                    "mode" => words.len() <= 8 && (row.effect != "mode-build" || !question),
                    "gate" => {
                        if !starts.contains(&hit.start) {
                            return false;
                        }
                        if (row.effect == "act" || row.effect == "go-on") && (!held || card_open || question) {
                            return false;
                        }
                        !(clean_phrase(&hit.phrase) == "continue" && hit.end != words.len())
                    }
                    _ => true,
                }
            })
            .collect();
        let mut covers: HashMap<usize, usize> = HashMap::new();
        for hit in &eligible {
            let end = covers.entry(hit.start).or_insert(0);
            *end = (*end).max(hit.end);
        }
        let longest: Vec<Hit> = eligible
            .into_iter()
            .filter(|hit| {
                (hit.end.saturating_sub(self.max_words)..=hit.start).all(|at| {
                    let end = covers.get(&at).copied().unwrap_or(0);
                    !(end >= hit.end && (at < hit.start || end > hit.end))
                })
            })
            .collect();
        let mut by_id: BTreeMap<String, Hit> = BTreeMap::new();
        for hit in longest {
            let id = self.keyword(&hit).id.clone();
            let longer = by_id.get(&id).is_none_or(|old| hit.end - hit.start > old.end - old.start);
            if longer {
                by_id.insert(id, hit);
            }
        }
        let mut unique: Vec<Hit> = by_id.into_values().collect();
        unique.sort_by(|a, b| {
            let (ka, kb) = (self.keyword(a), self.keyword(b));
            priority(kb).cmp(&priority(ka)).then((b.end - b.start).cmp(&(a.end - a.start))).then(a.start.cmp(&b.start)).then(ka.id.cmp(&kb.id))
        });
        for (less, more) in LESS {
            if unique.iter().any(|h| self.keyword(h).id == more)
                && let Some(at) = unique.iter().position(|h| self.keyword(h).id == less)
            {
                unique.remove(at);
            }
        }
        unique
    }

    pub fn lookup(&self, kind: &str, capability: &str) -> DecisionRow {
        let get = |k: &str, c: &str| self.table.get(&(k.to_string(), c.to_string())).cloned();
        get(kind, capability)
            .or_else(|| get(kind, "*"))
            .or_else(|| get("general", capability))
            .or_else(|| get("general", "*"))
            .unwrap_or_else(|| DecisionRow { kind: "general".into(), capability: "*".into(), mode: "build".into(), tier: "standard".into(), fallback: "secondary".into() })
    }

    pub fn decide(&self, text: &str, input: &Input) -> Result<FastDecision, String> {
        let hits = self.find(text, input.held, input.card_open);
        let has = |effects: &[&str]| hits.iter().any(|h| effects.contains(&self.keyword(h).effect.as_str()));
        let kind = input.kind.clone().unwrap_or_else(|| {
            if has(&["review-now", "second-opinion", "mode-review"]) {
                "review"
            } else if has(&["plan-topic", "plan-write", "mode-plan", "explain", "debate"]) {
                "planning"
            } else if has(&["test-first"]) {
                "testing"
            } else {
                "general"
            }
            .to_string()
        });
        let capability = input.capability.clone().unwrap_or_else(|| {
            match kind.as_str() {
                "planning" => "planning",
                "review" | "testing" => "testing",
                _ => "coding",
            }
            .to_string()
        });
        let row = self.lookup(&kind, &capability);
        let talk = has(&TALK);
        let review = has(&["review-now", "mode-review"]);
        let mode = if talk {
            "plan".to_string()
        } else if review {
            "review".to_string()
        } else if has(&["act", "mode-build"]) {
            "build".to_string()
        } else {
            input.mode.clone().filter(|m| MODES.contains(&m.as_str())).unwrap_or_else(|| row.mode.clone())
        };
        let risk = hits.iter().any(|h| self.keyword(h).place == "risk") || risky_text(text);
        let facts: BTreeMap<String, bool> = [
            ("stop", has(&["stop"])),
            ("hold", has(&["hold"]) || (input.held && !has(&["act", "go-on"]))),
            ("risk", risk),
            ("talk", mode == "plan"),
            ("review", mode == "review"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let path = self.trees.get(ROUTING).ok_or("Missing brain-routing tree.")?.run(&facts)?;
        let tier = if has(&["cheap"]) && !risk {
            "cheap".to_string()
        } else if has(&["deeper", "best-model"]) {
            "premium".to_string()
        } else {
            row.tier.clone()
        };
        Ok(FastDecision {
            kind,
            capability,
            mode,
            tier,
            fallback: row.fallback,
            primary: hits.first().map(|h| self.keyword(h).id.clone()),
            keywords: hits.iter().map(|h| self.keyword(h).id.clone()).collect(),
            matches: hits.iter().map(|h| MatchView { id: self.keyword(h).id.clone(), phrase: h.phrase.clone(), start: h.start, end: h.end }).collect(),
            requires_approval: risk,
            grants_approval: false,
            action: path.action,
            path: path.path,
            comparisons: path.comparisons,
        })
    }

    pub fn tree(&self, name: &str, facts: &BTreeMap<String, bool>) -> Option<Result<super::tree::Run, String>> {
        self.trees.get(name).map(|t| t.run(facts))
    }

    pub fn self_check(&self) -> SelfCheck {
        let mut problems = Vec::new();
        for row in self.keywords.iter().filter(|k| k.on) {
            for phrase in &row.aliases {
                if !self.find(phrase, true, false).iter().any(|h| self.keyword(h).id == row.id) {
                    problems.push(format!("Unreachable keyword {}: {phrase}", row.id));
                }
            }
        }
        SelfCheck {
            ok: problems.is_empty(),
            problems,
            keywords: self.keywords.len(),
            phrases: self.phrases,
            table: self.table.len(),
            trees: self.trees.iter().map(|(name, t)| TreeDepth { name: name.clone(), depth: t.depth }).collect(),
            trie_states: self.index.states(),
        }
    }
}
