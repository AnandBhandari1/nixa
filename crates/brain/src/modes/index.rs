use crate::settings_api::keywords::phrase;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub item: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Default)]
struct Node {
    next: HashMap<String, usize>,
    fail: usize,
    output: Vec<(usize, usize)>,
}

pub struct PhraseIndex {
    nodes: Vec<Node>,
}

pub fn build(phrases: &[(String, usize)]) -> PhraseIndex {
    let mut nodes = vec![Node::default()];
    for (text, item) in phrases {
        let words: Vec<String> = phrase(text).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        if words.is_empty() {
            continue;
        }
        let mut at = 0;
        for word in &words {
            at = match nodes[at].next.get(word) {
                Some(next) => *next,
                None => {
                    let next = nodes.len();
                    nodes[at].next.insert(word.clone(), next);
                    nodes.push(Node::default());
                    next
                }
            };
        }
        nodes[at].output.push((*item, words.len()));
    }
    let mut queue: Vec<usize> = nodes[0].next.values().copied().collect();
    let mut head = 0;
    while head < queue.len() {
        let parent = queue[head];
        head += 1;
        let children: Vec<(String, usize)> = nodes[parent].next.iter().map(|(w, c)| (w.clone(), *c)).collect();
        for (word, child) in children {
            queue.push(child);
            let mut fail = nodes[parent].fail;
            while fail != 0 && !nodes[fail].next.contains_key(&word) {
                fail = nodes[fail].fail;
            }
            let target = nodes[fail].next.get(&word).copied().filter(|t| *t != child).unwrap_or(0);
            nodes[child].fail = target;
            let more = nodes[target].output.clone();
            nodes[child].output.extend(more);
        }
    }
    PhraseIndex { nodes }
}

pub fn scan(index: &PhraseIndex, text: &str) -> Vec<Hit> {
    let clean = phrase(text);
    let mut hits = Vec::new();
    let mut state = 0;
    for (at, word) in clean.split(' ').filter(|w| !w.is_empty()).enumerate() {
        while state != 0 && !index.nodes[state].next.contains_key(word) {
            state = index.nodes[state].fail;
        }
        state = index.nodes[state].next.get(word).copied().unwrap_or(0);
        for (item, length) in &index.nodes[state].output {
            hits.push(Hit { item: *item, start: at + 1 - length, end: at + 1 });
        }
    }
    hits
}

pub fn states(index: &PhraseIndex) -> usize {
    index.nodes.len()
}
