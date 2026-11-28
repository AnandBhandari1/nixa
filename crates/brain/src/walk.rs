use crate::trees::{Check, Condition, RuleTree, SLOT};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const RESEND_MS: u64 = 30 * 60_000;
pub const RETRY_MS: u64 = 4 * 60_000;
const MAX_STEPS: usize = 80;

pub type Facts = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct Subject {
    pub key: String,
    pub label: String,
    pub facts: Facts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub key: String,
    pub rule: String,
    pub text: String,
    pub fingerprint: String,
    pub retry: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Walk {
    pub end: String,
    pub finding: Option<Finding>,
}

fn same(value: Option<&Value>, wanted: &Value) -> bool {
    match value {
        None | Some(Value::Null) => wanted == &Value::String(String::new()) || wanted.is_null(),
        Some(v) => v == wanted,
    }
}

fn holds(facts: &Facts, condition: &Condition) -> bool {
    let value = facts.get(&condition.fact);
    if let Some(is) = &condition.is {
        return match is {
            Value::Array(list) => list.iter().any(|item| same(value, item)),
            other => same(value, other),
        };
    }
    if let Some(list) = &condition.within {
        return list.iter().any(|item| same(value, item));
    }
    let Some(number) = value.and_then(Value::as_f64) else { return false };
    if let Some(gt) = condition.gt {
        return number > gt;
    }
    if let Some(gte) = condition.gte {
        return number >= gte;
    }
    if let Some(lt) = condition.lt {
        return number < lt;
    }
    if let Some(lte) = condition.lte {
        return number <= lte;
    }
    false
}

fn plain(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

pub fn pick_line(check: &Check, facts: &Facts) -> String {
    let yes = |ok: bool| if ok { "Yes".to_string() } else { "No".to_string() };
    match check {
        Check::Cases { fact, cases, otherwise } => {
            let value = plain(facts.get(fact));
            cases.get(&value).cloned().or_else(|| otherwise.clone()).unwrap_or_default()
        }
        Check::All { all } => yes(all.iter().all(|c| holds(facts, c))),
        Check::Any { any } => yes(any.iter().any(|c| holds(facts, c))),
        Check::One(c) => yes(holds(facts, c)),
    }
}

fn facts_of(check: &Check) -> Vec<String> {
    match check {
        Check::Cases { fact, .. } => vec![fact.clone()],
        Check::All { all } => all.iter().map(|c| c.fact.clone()).collect(),
        Check::Any { any } => any.iter().map(|c| c.fact.clone()).collect(),
        Check::One(c) => vec![c.fact.clone()],
    }
}

fn fill(text: &str, subject: &Subject) -> String {
    SLOT.replace_all(text, |caps: &regex::Captures| {
        let name = &caps[1];
        if name == "item" {
            return subject.label.clone();
        }
        match subject.facts.get(name) {
            None | Some(Value::Null) => "none".to_string(),
            Some(v) => plain(Some(v)),
        }
    })
    .into_owned()
}

fn fingerprint(subject: &Subject, rule: &str, used: &HashSet<String>) -> String {
    let mut stable: Vec<&String> = used.iter().filter(|n| !n.to_lowercase().contains("minutes") && !n.to_lowercase().contains("since")).collect();
    stable.sort();
    let pairs: Vec<Value> = stable.iter().map(|n| Value::Array(vec![Value::String((*n).clone()), subject.facts.get(*n).cloned().unwrap_or(Value::Null)])).collect();
    serde_json::to_string(&(rule, pairs)).unwrap_or_default()
}

fn strip_report(text: &str) -> &str {
    let trimmed = text.trim_start();
    match (trimmed.get(..7), trimmed.get(7..)) {
        (Some(head), Some(rest)) if head.eq_ignore_ascii_case("report:") => rest.trim_start(),
        _ => text,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub at: String,
    pub text: String,
    pub pick: String,
    pub to: String,
}

pub fn walk_path(tree: &RuleTree, subject: &Subject) -> Walk {
    walk_steps(tree, subject).0
}

pub fn walk_steps(tree: &RuleTree, subject: &Subject) -> (Walk, Vec<Step>) {
    let mut used: HashSet<String> = HashSet::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut id = tree.start.clone();
    for _ in 0..MAX_STEPS {
        let Some(rule) = tree.boxes.get(&id) else { return (Walk { end: id, finding: None }, steps) };
        if rule.report {
            for slot in SLOT.captures_iter(&rule.text) {
                if &slot[1] != "item" {
                    used.insert(slot[1].to_string());
                }
            }
            let text = fill(strip_report(&rule.text), subject);
            let finding = Finding { key: subject.key.clone(), rule: id.clone(), text, fingerprint: fingerprint(subject, &id, &used), retry: false };
            return (Walk { end: id, finding: Some(finding) }, steps);
        }
        let Some(check) = &rule.check else { return (Walk { end: id, finding: None }, steps) };
        used.extend(facts_of(check));
        let word = pick_line(check, &subject.facts);
        let to = rule.next.as_ref().and_then(|n| n.get(&word)).cloned();
        steps.push(Step { at: id.clone(), text: rule.text.clone(), pick: word, to: to.clone().unwrap_or_default() });
        match to {
            Some(to) => id = to,
            None => return (Walk { end: id, finding: None }, steps),
        }
    }
    (Walk { end: id, finding: None }, steps)
}

pub fn walks_json(tree: &RuleTree, subjects: &[Subject]) -> Value {
    let walks: Vec<Value> = subjects
        .iter()
        .map(|s| {
            let (walk, steps) = walk_steps(tree, s);
            let end_text = tree.boxes.get(&walk.end).map(|b| b.text.clone()).unwrap_or_default();
            json!({
                "key": s.key,
                "steps": steps,
                "end": walk.end,
                "end_text": end_text,
                "finding": walk.finding.as_ref().map(|f| json!({"rule": f.rule, "text": f.text})),
            })
        })
        .collect();
    json!({"tree": tree.name, "start": tree.start, "walks": walks})
}

pub fn walk_tree(tree: &RuleTree, subjects: &[Subject]) -> Vec<Finding> {
    subjects.iter().filter_map(|s| walk_path(tree, s).finding).collect()
}

pub fn dry_run(tree: &RuleTree, subjects: &[Subject]) -> String {
    let walks: Vec<Walk> = subjects.iter().map(|s| walk_path(tree, s)).collect();
    let findings: Vec<&Finding> = walks.iter().filter_map(|w| w.finding.as_ref()).collect();
    let mut ends: BTreeMap<String, usize> = BTreeMap::new();
    for walk in &walks {
        *ends.entry(walk.end.clone()).or_default() += 1;
    }
    let mut ends: Vec<(String, usize)> = ends.into_iter().collect();
    ends.sort_by_key(|a| std::cmp::Reverse(a.1));
    let end_line: Vec<String> = ends.iter().map(|(b, n)| format!("{b} x {n}")).collect();
    let mut out = vec![format!(
        "Dry run of {}: {} items walked, {} finding{}. Nothing was saved or sent.",
        tree.name,
        subjects.len(),
        findings.len(),
        if findings.len() == 1 { "" } else { "s" }
    )];
    out.extend(findings.iter().map(|f| format!("- {} ends at {}: {}", f.key, f.rule, f.text)));
    out.push(format!("Walks ended at: {}.", if end_line.is_empty() { "none".to_string() } else { end_line.join(", ") }));
    out.join("\n")
}

#[derive(Debug, Default)]
pub struct WatchMemory {
    sent: HashMap<String, (String, u64, u32)>,
}

impl WatchMemory {
    pub fn pick(&mut self, findings: &[Finding], at: u64) -> Vec<Finding> {
        let live: HashSet<String> = findings.iter().map(|f| format!("{}|{}", f.key, f.rule)).collect();
        self.sent.retain(|key, (_, when, _)| live.contains(key) || at.saturating_sub(*when) < RESEND_MS);
        let mut out = Vec::new();
        for finding in findings {
            let key = format!("{}|{}", finding.key, finding.rule);
            if let Some((print, when, tries)) = self.sent.get(&key).cloned()
                && print == finding.fingerprint {
                    if tries == 1 && at.saturating_sub(when) >= RETRY_MS {
                        self.sent.insert(key, (finding.fingerprint.clone(), at, 2));
                        out.push(Finding { retry: true, ..finding.clone() });
                        continue;
                    }
                    if at.saturating_sub(when) < RESEND_MS {
                        continue;
                    }
                }
            self.sent.insert(key, (finding.fingerprint.clone(), at, 1));
            out.push(finding.clone());
        }
        out
    }

    pub fn forget(&mut self, findings: &[Finding]) {
        for f in findings {
            self.sent.remove(&format!("{}|{}", f.key, f.rule));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trees::{WATCH_TREE, check_tree, default_watch_tree};
    use serde_json::json;

    #[test]
    fn strip_report_is_safe_on_wide_chars() {
        assert_eq!(strip_report("🚨🚨 {item}"), "🚨🚨 {item}");
        assert_eq!(strip_report("Report: {item} waits"), "{item} waits");
        assert_eq!(strip_report("ab"), "ab");
        assert_eq!(strip_report("repor€: x"), "repor€: x");
    }

    fn subject(status: &str, minutes: u64, job: bool) -> Subject {
        let mut facts = Facts::new();
        facts.insert("subject".into(), json!("terminal"));
        facts.insert("terminal.name".into(), json!("claude"));
        facts.insert("terminal.status".into(), json!(status));
        facts.insert("terminal.note".into(), json!("needs a yes"));
        facts.insert("terminal.minutesInStatus".into(), json!(minutes));
        facts.insert("terminal.nixaJob".into(), json!(job));
        Subject { key: "terminal m3".into(), label: "terminal m3".into(), facts }
    }

    #[test]
    fn walks_default_tree() {
        let tree = check_tree(&default_watch_tree(), Some(WATCH_TREE)).unwrap();
        let blocked = walk_path(&tree, &subject("blocked", 1, false)).finding.unwrap();
        assert_eq!(blocked.text, "terminal m3 (claude) is blocked and waits for an answer: needs a yes.");
        assert!(walk_path(&tree, &subject("working", 5, false)).finding.is_none());
        assert_eq!(walk_path(&tree, &subject("working", 45, false)).finding.unwrap().rule, "r6");
        assert_eq!(walk_path(&tree, &subject("done", 0, true)).finding.unwrap().rule, "r8");
        assert_eq!(walk_path(&tree, &subject("idle", 0, true)).end, "r9");
        let text = dry_run(&tree, &[subject("blocked", 1, false), subject("idle", 0, false)]);
        assert!(text.contains("2 items walked, 1 finding."));
        let (walk, steps) = walk_steps(&tree, &subject("working", 45, false));
        assert_eq!(walk.end, "r6");
        assert_eq!(steps.first().map(|s| s.at.as_str()), Some(tree.start.as_str()));
        assert!(steps.iter().all(|s| !s.pick.is_empty() && !s.to.is_empty()));
        let shown = walks_json(&tree, &[subject("blocked", 1, false)]);
        assert_eq!(shown["walks"][0]["key"], json!("terminal m3"));
        assert!(shown["walks"][0]["finding"]["text"].as_str().unwrap().contains("blocked"));
    }

    #[test]
    fn memory_sends_once_then_retries() {
        let tree = check_tree(&default_watch_tree(), Some(WATCH_TREE)).unwrap();
        let found = walk_tree(&tree, &[subject("working", 45, false)]);
        let mut memory = WatchMemory::default();
        assert_eq!(memory.pick(&found, 0).len(), 1);
        let later = walk_tree(&tree, &[subject("working", 46, false)]);
        assert!(memory.pick(&later, 60_000).is_empty());
        let retry = memory.pick(&later, RETRY_MS + 1);
        assert!(retry[0].retry);
        assert!(memory.pick(&later, RETRY_MS + 2).is_empty());
        memory.forget(&later);
        assert_eq!(memory.pick(&later, RETRY_MS + 3).len(), 1);
    }
}
