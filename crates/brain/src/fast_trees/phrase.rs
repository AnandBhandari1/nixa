use std::collections::HashMap;

pub fn clean_phrase(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.to_lowercase().chars() {
        if c == '\'' || c == '\u{2019}' {
            continue;
        }
        out.push(if c.is_alphanumeric() { c } else { ' ' });
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn words_of(text: &str) -> Vec<String> {
    clean_phrase(text).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub item: usize,
    pub phrase: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
struct Out {
    item: usize,
    phrase: String,
    length: usize,
}

#[derive(Debug, Clone, Default)]
struct Node {
    next: HashMap<String, usize>,
    fail: usize,
    out: Vec<Out>,
}

#[derive(Debug, Clone)]
pub struct PhraseIndex {
    nodes: Vec<Node>,
}

impl PhraseIndex {
    pub fn new(phrases: &[(String, usize)]) -> Result<PhraseIndex, String> {
        let mut nodes = vec![Node::default()];
        for (phrase, item) in phrases {
            let words = words_of(phrase);
            if words.is_empty() {
                return Err("A keyword phrase is empty.".into());
            }
            let mut at = 0;
            for word in words.iter() {
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
            nodes[at].out.push(Out { item: *item, phrase: phrase.clone(), length: words.len() });
        }
        let mut queue: Vec<usize> = nodes[0].next.values().copied().collect();
        let mut index = 0;
        while index < queue.len() {
            let parent = queue[index];
            index += 1;
            let children: Vec<(String, usize)> = nodes[parent].next.iter().map(|(w, c)| (w.clone(), *c)).collect();
            for (word, child) in children {
                queue.push(child);
                let mut fail = nodes[parent].fail;
                while fail != 0 && !nodes[fail].next.contains_key(&word) {
                    fail = nodes[fail].fail;
                }
                let target = nodes[fail].next.get(&word).copied().filter(|t| *t != child).unwrap_or(0);
                nodes[child].fail = target;
                let more = nodes[target].out.clone();
                nodes[child].out.extend(more);
            }
        }
        Ok(PhraseIndex { nodes })
    }

    pub fn scan(&self, text: &str) -> (Vec<String>, Vec<Hit>) {
        let words = words_of(text);
        let mut hits = Vec::new();
        let mut state = 0;
        for (at, word) in words.iter().enumerate() {
            while state != 0 && !self.nodes[state].next.contains_key(word) {
                state = self.nodes[state].fail;
            }
            state = self.nodes[state].next.get(word).copied().unwrap_or(0);
            for out in &self.nodes[state].out {
                hits.push(Hit { item: out.item, phrase: out.phrase.clone(), start: at + 1 - out.length, end: at + 1 });
            }
        }
        (words, hits)
    }

    pub fn states(&self) -> usize {
        self.nodes.len()
    }
}
