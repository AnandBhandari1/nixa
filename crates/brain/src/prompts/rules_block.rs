use crate::trees::{RuleTree, TreeInfo};
use crate::util::clip_flat;

pub const HEAD: &str = "Rules (the user's rules, all in one place: standing rules, rule trees and answer rules):";

pub fn answer_line(count: usize) -> String {
    format!("Answer rules: {count} saved in answer-rules.json. Nixa applies them itself: when an agent asks a question that fits one, Nixa answers it with the rule's answer. List or change them with nixa_cli rules answers.")
}

pub fn block(soul: &str, trees: &[TreeInfo], full: &[RuleTree], answers: usize) -> String {
    let mut out = Vec::new();
    if !soul.is_empty() {
        out.push(soul.to_string());
    }
    if !trees.is_empty() {
        out.push(format!("Rule trees (JSON):\n{}", crate::trees::trees_json(full, crate::trees::PROMPT_MAX)));
        for tree in trees {
            if let Some(problem) = &tree.broken {
                out.push(format!("- {} is broken: {}", tree.name, clip_flat(problem, 200)));
            }
        }
    }
    if answers > 0 {
        out.push(answer_line(answers));
    }
    if out.is_empty() {
        return String::new();
    }
    out.insert(0, HEAD.to_string());
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_rules_block_in_order() {
        assert_eq!(block("", &[], &[], 0), "");
        let tree: RuleTree = serde_json::from_value(crate::trees::default_question_tree()).unwrap();
        let info = TreeInfo { name: tree.name.clone(), title: tree.title.clone(), when: tree.when.clone(), boxes: tree.boxes.len(), broken: Some("bad box".into()) };
        let text = block("Standing rules:\nshort words", &[info], &[tree], 2);
        assert!(text.starts_with(HEAD));
        let soul = text.find("short words").unwrap();
        let trees = text.find("Rule trees (JSON)").unwrap();
        let broken = text.find("is broken: bad box").unwrap();
        let answers = text.find("Answer rules: 2 saved").unwrap();
        assert!(soul < trees && trees < broken && broken < answers);
        assert!(block("x", &[], &[], 0).ends_with("x"));
    }
}
