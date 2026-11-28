use serde_json::{Value, json};

pub const PLACES: [&str; 5] = ["sofa", "tea", "window", "desk", "meeting"];
pub const ACTS: [&str; 5] = ["none", "wave", "laugh", "nod", "sip"];
pub const CAST_MIN: usize = 2;
pub const CAST_MAX: usize = 8;
pub const WORK_MAX: usize = 6;
pub const TASK_CHARS: usize = 60;
pub const WORD_CHARS: usize = 40;
pub const TEXT_CHARS: usize = 90;
pub const MOOD_CHARS: usize = 20;
pub const LINES_MIN: usize = 2;
pub const LINES_MAX: usize = 8;
pub const SCENE_CAST_MIN: usize = 2;
pub const SCENE_CAST_MAX: usize = 3;

pub const SYSTEM: &str = "You write one short scene of a friendly, cozy office sitcom.
Use plain simple English. Each line is at most 12 words.
People talk to each other by first name.
Nobody says their own name. Never start a line with a name and a colon.
Use light jokes, small talk, tea, the weekend, and light gossip about the given task titles.
Keep it office safe. No code, no secrets, and no real people outside the cast.
Pick 2 or 3 people from the cast. Only they speak.
Pick where from: sofa, tea, window, desk, meeting.
Pick each act from: none, wave, laugh, nod, sip.
Write 2 to 8 lines. Continue the running story when it fits.
Answer with JSON only.";

#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub role: String,
    pub space: String,
    pub status: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Work {
    pub who: String,
    pub name: String,
    pub task: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub cast: Vec<Person>,
    pub work: Vec<Work>,
    pub hour: u8,
}

pub fn cut(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max - 1).collect::<String>().trim_end().to_string();
    out.push('…');
    out
}

fn word(value: &Value, key: &str) -> String {
    cut(value.get(key).and_then(Value::as_str).unwrap_or(""), WORD_CHARS)
}

pub fn parse_ask(value: &Value) -> Result<Ask, String> {
    let mut cast: Vec<Person> = Vec::new();
    for item in value.get("cast").and_then(Value::as_array).into_iter().flatten() {
        let person = Person {
            id: word(item, "id"),
            name: word(item, "name"),
            role: word(item, "role"),
            space: word(item, "space"),
            status: word(item, "status"),
            at: word(item, "at"),
        };
        if person.id.is_empty() || cast.iter().any(|p| p.id == person.id) {
            continue;
        }
        let person = Person { name: if person.name.is_empty() { person.id.clone() } else { person.name }, ..person };
        cast.push(person);
        if cast.len() == CAST_MAX {
            break;
        }
    }
    if cast.len() < CAST_MIN {
        return Err(format!("The cast needs {CAST_MIN} to {CAST_MAX} people."));
    }
    let work = value
        .get("work")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let task = cut(item.get("task").and_then(Value::as_str).unwrap_or(""), TASK_CHARS);
            (!task.is_empty()).then(|| Work { who: word(item, "who"), name: word(item, "name"), task })
        })
        .take(WORK_MAX)
        .collect();
    let hour = value.get("hour").and_then(Value::as_u64).filter(|h| *h < 24).unwrap_or(12) as u8;
    Ok(Ask { cast, work, hour })
}

pub fn user_message(ask: &Ask, memory: &[String], said: &[String]) -> String {
    let mut lines = vec!["Cast:".to_string()];
    for p in &ask.cast {
        lines.push(format!("- id {}: {} ({}, space {}, {}, at the {})", p.id, p.name, p.role, p.space, p.status, p.at));
    }
    lines.push(String::new());
    if ask.work.is_empty() {
        lines.push("Work today: nothing listed.".into());
    } else {
        lines.push("Work today:".into());
        for w in &ask.work {
            lines.push(format!("- {} works on \"{}\"", if w.name.is_empty() { &w.who } else { &w.name }, w.task));
        }
    }
    lines.push(String::new());
    lines.push(format!("The hour is {}:00.", ask.hour));
    lines.push(String::new());
    if memory.is_empty() {
        lines.push("Story so far today: this is the first scene.".into());
    } else {
        lines.push("Story so far today:".into());
        lines.extend(memory.iter().map(|m| format!("- {m}")));
    }
    if !said.is_empty() {
        lines.push(String::new());
        lines.push("Lines already said today (never reuse them):".into());
        lines.extend(said.iter().map(|m| format!("- {m}")));
    }
    lines.push(String::new());
    lines.push("Write the next scene. Use the ids in cast and who.".into());
    lines.join("\n")
}

pub fn schema(ask: &Ask) -> Value {
    let ids: Vec<&str> = ask.cast.iter().map(|p| p.id.as_str()).collect();
    json!({
        "type": "object",
        "properties": {
            "where": {"type": "string", "enum": PLACES},
            "cast": {"type": "array", "items": {"type": "string", "enum": ids}, "minItems": SCENE_CAST_MIN, "maxItems": SCENE_CAST_MAX},
            "mood": {"type": "string"},
            "lines": {
                "type": "array",
                "minItems": LINES_MIN,
                "maxItems": LINES_MAX,
                "items": {
                    "type": "object",
                    "properties": {
                        "who": {"type": "string", "enum": ids},
                        "text": {"type": "string"},
                        "act": {"type": "string", "enum": ACTS}
                    },
                    "required": ["who", "text", "act"]
                }
            }
        },
        "required": ["where", "cast", "mood", "lines"]
    })
}

pub fn body(model: &str, ask: &Ask, memory: &[String], said: &[String]) -> Value {
    json!({
        "model": model,
        "stream": false,
        "think": false,
        "keep_alive": "2m",
        "format": schema(ask),
        "options": {"temperature": 0.9, "num_predict": 400},
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": user_message(ask, memory, said)}
        ]
    })
}

fn pick(value: Option<&Value>, list: &[&str], fallback: &str) -> String {
    let got = value.and_then(Value::as_str).map(|s| s.trim().to_lowercase()).unwrap_or_default();
    if list.contains(&got.as_str()) { got } else { fallback.to_string() }
}

fn same_word(word: &str, name: &str) -> bool {
    word.trim_matches(|c: char| !c.is_alphanumeric()).eq_ignore_ascii_case(name)
}

pub fn no_own_name(text: &str, own: &str, names: &[&str]) -> String {
    let mut rest = text.trim();
    if let Some((head, tail)) = rest.split_once(':')
        && names.iter().chain(std::iter::once(&own)).any(|n| !n.is_empty() && head.trim().eq_ignore_ascii_case(n))
    {
        rest = tail.trim();
    }
    let own = own.trim();
    if own.is_empty() {
        return rest.to_string();
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    for word in words {
        if same_word(word, own) {
            let end: String = word.chars().rev().take_while(|c| matches!(c, '?' | '!' | '.')).collect();
            if let Some(last) = out.last_mut() {
                let trimmed = last.trim_end_matches(',').to_string();
                *last = format!("{trimmed}{}", end.chars().rev().collect::<String>());
            }
            continue;
        }
        out.push(word.to_string());
    }
    let joined = out.join(" ");
    let joined = joined.trim_start_matches([',', ' ']).trim();
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn clean(raw: &Value, ask: &Ask) -> Result<Value, String> {
    let known = |id: &str| ask.cast.iter().any(|p| p.id == id);
    let names: Vec<&str> = ask.cast.iter().map(|p| p.name.as_str()).collect();
    let own = |id: &str| ask.cast.iter().find(|p| p.id == id).map(|p| p.name.as_str()).unwrap_or("");
    let mut lines: Vec<(String, String, String)> = Vec::new();
    for line in raw.get("lines").and_then(Value::as_array).into_iter().flatten() {
        let who = line.get("who").and_then(Value::as_str).map(str::trim).unwrap_or("");
        let text = cut(&no_own_name(line.get("text").and_then(Value::as_str).unwrap_or(""), own(who), &names), TEXT_CHARS);
        if !known(who) || text.is_empty() {
            continue;
        }
        lines.push((who.to_string(), text, pick(line.get("act"), &ACTS, "none")));
    }
    let mut cast: Vec<String> = Vec::new();
    for (who, _, _) in &lines {
        if !cast.contains(who) && cast.len() < SCENE_CAST_MAX {
            cast.push(who.clone());
        }
    }
    for id in raw.get("cast").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::trim) {
        if known(id) && !cast.iter().any(|c| c == id) && cast.len() < SCENE_CAST_MAX {
            cast.push(id.to_string());
        }
    }
    lines.retain(|(who, _, _)| cast.contains(who));
    lines.truncate(LINES_MAX);
    if cast.len() < SCENE_CAST_MIN {
        return Err("The model wrote a scene with fewer than 2 known people.".into());
    }
    if lines.len() < LINES_MIN {
        return Err("The model wrote fewer than 2 usable lines.".into());
    }
    let mood = cut(raw.get("mood").and_then(Value::as_str).unwrap_or(""), MOOD_CHARS);
    Ok(json!({
        "where": pick(raw.get("where"), &PLACES, "sofa"),
        "cast": cast,
        "mood": if mood.is_empty() { "cozy".to_string() } else { mood },
        "lines": lines.iter().map(|(who, text, act)| json!({"who": who, "text": text, "act": act})).collect::<Vec<_>>()
    }))
}

pub fn parse_reply(reply: &Value, ask: &Ask) -> Result<Value, String> {
    let content = reply
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .ok_or("Ollama sent no message.")?;
    let raw: Value = serde_json::from_str(content.trim()).map_err(|_| "The model did not write valid JSON.".to_string())?;
    clean(&raw, ask)
}

pub fn summary(scene: &Value, ask: &Ask) -> String {
    let name = |id: &str| ask.cast.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
    let cast: Vec<String> = scene["cast"].as_array().into_iter().flatten().filter_map(Value::as_str).map(name).collect();
    let first = &scene["lines"][0];
    let line = format!("{}: {}", name(first["who"].as_str().unwrap_or("")), first["text"].as_str().unwrap_or(""));
    cut(&format!("{} at the {} ({}). {line}", cast.join(", "), scene["where"].as_str().unwrap_or(""), scene["mood"].as_str().unwrap_or("")), 160)
}

pub fn drop_repeats(scene: &mut Value, said: &[String]) {
    let old: Vec<String> = said.iter().map(|s| super::fun::norm(s)).collect();
    let Some(lines) = scene.get_mut("lines").and_then(Value::as_array_mut) else { return };
    let fresh: Vec<Value> = lines.iter().filter(|l| !old.contains(&super::fun::norm(l["text"].as_str().unwrap_or("")))).cloned().collect();
    if fresh.len() >= LINES_MIN {
        *lines = fresh;
    }
}
