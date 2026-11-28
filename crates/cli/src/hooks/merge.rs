use super::json::{self, Json};
use anyhow::{Result, anyhow, bail};

pub const SCRIPT: &str = "nixa-status.sh";

pub fn quote(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:@%,".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn first_word(cmd: &str) -> String {
    let mut word = String::new();
    let mut chars = cmd.trim_start().chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        c => word.push(c),
                    }
                }
            }
            '\\' => word.extend(chars.next()),
            c if c.is_whitespace() => break,
            c => word.push(c),
        }
    }
    word
}

fn command(hook: &Json) -> Option<&str> {
    match json::get(hook, "command") {
        Some(Json::Str(c)) => Some(c),
        _ => None,
    }
}

fn is_ours(hook: &Json, script: &str) -> bool {
    command(hook).is_some_and(|c| first_word(c) == script)
}

fn is_status_hook(hook: &Json) -> bool {
    command(hook).is_some_and(|c| first_word(c).ends_with(&format!("/{SCRIPT}")))
}

pub fn rewrite(template: &mut Json, script: &str) -> Result<()> {
    let broken = || anyhow!("the hook template has the wrong shape");
    let Some(Json::Obj(events)) = json::get_mut(template, "hooks") else { return Err(broken()) };
    for (_, groups) in events.iter_mut() {
        let Json::Arr(groups) = groups else { return Err(broken()) };
        for group in groups {
            if !matches!(json::get(group, "hooks"), Some(Json::Arr(_))) {
                set_script(group, script)?;
                continue;
            }
            let Some(Json::Arr(hooks)) = json::get_mut(group, "hooks") else { return Err(broken()) };
            for hook in hooks {
                set_script(hook, script)?;
            }
        }
    }
    Ok(())
}

fn set_script(hook: &mut Json, script: &str) -> Result<()> {
    let broken = || anyhow!("the hook template has the wrong shape");
    let Some(Json::Str(cmd)) = json::get_mut(hook, "command") else { return Err(broken()) };
    let args = cmd.split_once(SCRIPT).map(|(_, a)| a.trim().to_string()).ok_or_else(broken)?;
    *cmd = if args.is_empty() { quote(script) } else { format!("{} {args}", quote(script)) };
    Ok(())
}

pub fn add(root: &mut Json, template: &Json) -> Result<()> {
    if !matches!(root, Json::Obj(_)) {
        bail!("the file is not a JSON object");
    }
    let Some(Json::Obj(events)) = json::get(template, "hooks") else { bail!("the hook template has no hooks") };
    let hooks = json::entry(root, "hooks", Json::Obj(Vec::new())).ok_or_else(|| anyhow!("the file is not a JSON object"))?;
    if !matches!(hooks, Json::Obj(_)) {
        bail!("\"hooks\" in the file is not an object");
    }
    for (event, groups) in events {
        let Json::Arr(groups) = groups else { bail!("the hook template has the wrong shape") };
        match json::entry(hooks, event, Json::Arr(Vec::new())) {
            Some(Json::Arr(list)) => list.extend(groups.iter().cloned()),
            _ => bail!("\"hooks.{event}\" in the file is not a list"),
        }
    }
    Ok(())
}

pub fn remove(root: &mut Json, script: &str) {
    let Some(Json::Obj(events)) = json::get_mut(root, "hooks") else { return };
    let mut emptied = Vec::new();
    let mut any = false;
    for (event, list) in events.iter_mut() {
        let Json::Arr(groups) = list else { continue };
        let mut touched = false;
        groups.retain_mut(|group| {
            if !matches!(json::get(group, "hooks"), Some(Json::Arr(_))) {
                let mine = is_ours(group, script);
                touched |= mine;
                return !mine;
            }
            let Some(Json::Arr(hooks)) = json::get_mut(group, "hooks") else { return true };
            let before = hooks.len();
            hooks.retain(|h| !is_ours(h, script));
            if hooks.len() == before {
                return true;
            }
            touched = true;
            !hooks.is_empty()
        });
        any |= touched;
        if touched && groups.is_empty() {
            emptied.push(event.clone());
        }
    }
    events.retain(|(event, _)| !emptied.contains(event));
    let none_left = events.is_empty();
    if any && none_left {
        json::drop_key(root, "hooks");
    }
}

pub fn ours(root: &Json, script: &str) -> Vec<String> {
    pick(root, &|h| is_ours(h, script))
}

pub fn others(root: &Json, script: &str) -> Vec<String> {
    pick(root, &|h| is_status_hook(h) && !is_ours(h, script))
}

fn pick(root: &Json, keep: &dyn Fn(&Json) -> bool) -> Vec<String> {
    let mut found = Vec::new();
    let Some(Json::Obj(events)) = json::get(root, "hooks") else { return found };
    for (event, list) in events {
        let Json::Arr(groups) = list else { continue };
        for group in groups {
            let Some(Json::Arr(hooks)) = json::get(group, "hooks") else {
                if keep(group) {
                    found.push(format!("{event} {}", json::compact(group)));
                }
                continue;
            };
            let mine: Vec<Json> = hooks.iter().filter(|h| keep(h)).cloned().collect();
            if mine.is_empty() {
                continue;
            }
            let mut shown = group.clone();
            if let Some(slot) = json::get_mut(&mut shown, "hooks") {
                *slot = Json::Arr(mine);
            }
            found.push(format!("{event} {}", json::compact(&shown)));
        }
    }
    found
}

pub fn minus(a: &[String], b: &[String]) -> Vec<String> {
    let mut left: Vec<&String> = b.iter().collect();
    a.iter()
        .filter(|x| match left.iter().position(|y| y == x) {
            Some(at) => {
                left.remove(at);
                false
            }
            None => true,
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::json::{compact, parse};

    const SCRIPT_PATH: &str = "/x/hooks/nixa-status.sh";

    fn template() -> Json {
        let mut t = parse(r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/old/hooks/nixa-status.sh done","timeout":5}]}],"PreToolUse":[{"matcher":"*","hooks":[{"type":"command","command":"/old/hooks/nixa-status.sh blocked 'needs it'"}]}]}}"#).unwrap();
        rewrite(&mut t, SCRIPT_PATH).unwrap();
        t
    }

    #[test]
    fn quoting_round_trips() {
        for s in ["/a/b/nixa-status.sh", "/a b/it's/nixa-status.sh", "", "/x\"y"] {
            assert_eq!(first_word(&format!("{} working 'note'", quote(s))), s);
        }
        assert_eq!(quote("/a/b"), "/a/b");
        assert_eq!(quote("/a b"), "'/a b'");
        assert_eq!(first_word("  \"/a \\\" b\" x"), "/a \" b");
        assert_eq!(first_word("/a\\ b c"), "/a b");
    }

    #[test]
    fn rewrite_puts_our_script_in() {
        let t = template();
        assert_eq!(
            ours(&t, SCRIPT_PATH),
            vec![
                format!(r#"Stop {{"hooks":[{{"type":"command","command":"{SCRIPT_PATH} done","timeout":5}}]}}"#),
                format!(r#"PreToolUse {{"matcher":"*","hooks":[{{"type":"command","command":"{SCRIPT_PATH} blocked 'needs it'"}}]}}"#),
            ]
        );
    }

    #[test]
    fn add_then_remove_gives_back_the_file() {
        let start = r#"{"z":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say hi"}]}]},"a":2}"#;
        let mut root = parse(start).unwrap();
        add(&mut root, &template()).unwrap();
        let text = compact(&root);
        assert!(text.starts_with(r#"{"z":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say hi"}]},"#));
        assert!(text.contains(r#""PreToolUse":[{"matcher":"*""#));
        assert_eq!(ours(&root, SCRIPT_PATH).len(), 2);
        assert!(others(&root, SCRIPT_PATH).is_empty());
        remove(&mut root, SCRIPT_PATH);
        assert_eq!(compact(&root), start);
    }

    #[test]
    fn remove_keeps_other_hooks_in_a_shared_group() {
        let mut root = parse(&format!(r#"{{"hooks":{{"Stop":[{{"hooks":[{{"command":"say hi"}},{{"command":"{SCRIPT_PATH} done"}}]}}]}}}}"#)).unwrap();
        remove(&mut root, SCRIPT_PATH);
        assert_eq!(compact(&root), r#"{"hooks":{"Stop":[{"hooks":[{"command":"say hi"}]}]}}"#);
        let mut ours_only = template();
        remove(&mut ours_only, SCRIPT_PATH);
        assert_eq!(compact(&ours_only), "{}");
        let mut empty = parse(r#"{"hooks":{}}"#).unwrap();
        remove(&mut empty, SCRIPT_PATH);
        assert_eq!(compact(&empty), r#"{"hooks":{}}"#);
    }

    #[test]
    fn other_status_hooks_are_seen_not_touched() {
        let mut root = parse(r#"{"hooks":{"Stop":[{"hooks":[{"command":"/old/hooks/nixa-status.sh done"}]}]}}"#).unwrap();
        assert_eq!(others(&root, SCRIPT_PATH).len(), 1);
        remove(&mut root, SCRIPT_PATH);
        assert_eq!(others(&root, SCRIPT_PATH).len(), 1);
    }

    #[test]
    fn wrong_shapes_are_refused() {
        assert!(add(&mut parse("[]").unwrap(), &template()).is_err());
        assert!(add(&mut parse(r#"{"hooks":[]}"#).unwrap(), &template()).is_err());
        assert!(add(&mut parse(r#"{"hooks":{"Stop":{}}}"#).unwrap(), &template()).is_err());
        let mut t = parse(r#"{"hooks":{"Stop":[{"hooks":[{"command":"say hi"}]}]}}"#).unwrap();
        assert!(rewrite(&mut t, SCRIPT_PATH).is_err());
    }

    #[test]
    fn flat_lists_without_groups_work_too() {
        let mut t = parse(r#"{"version":1,"hooks":{"stop":[{"command":"./hooks/nixa-status.sh done"}],"sessionStart":[{"command":"./hooks/nixa-status.sh --resume cursor idle"}]}}"#).unwrap();
        rewrite(&mut t, SCRIPT_PATH).unwrap();
        assert_eq!(
            ours(&t, SCRIPT_PATH),
            vec![
                format!(r#"stop {{"command":"{SCRIPT_PATH} done"}}"#),
                format!(r#"sessionStart {{"command":"{SCRIPT_PATH} --resume cursor idle"}}"#),
            ]
        );
        let start = r#"{"version":1,"hooks":{"stop":[{"command":"say hi"}]}}"#;
        let mut root = parse(start).unwrap();
        add(&mut root, &t).unwrap();
        assert_eq!(ours(&root, SCRIPT_PATH).len(), 2);
        let mut other = parse(r#"{"hooks":{"stop":[{"command":"/old/hooks/nixa-status.sh done"}]}}"#).unwrap();
        assert_eq!(others(&other, SCRIPT_PATH).len(), 1);
        remove(&mut other, SCRIPT_PATH);
        assert_eq!(others(&other, SCRIPT_PATH).len(), 1);
        remove(&mut root, SCRIPT_PATH);
        assert_eq!(compact(&root), start);
        let mut bad = parse(r#"{"hooks":{"stop":[{"command":"say hi"}]}}"#).unwrap();
        assert!(rewrite(&mut bad, SCRIPT_PATH).is_err());
    }

    #[test]
    fn minus_counts_repeats() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(minus(&s(&["a", "a", "b"]), &s(&["a"])), s(&["a", "b"]));
        assert!(minus(&s(&["a"]), &s(&["a", "a"])).is_empty());
    }
}
