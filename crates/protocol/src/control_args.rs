use crate::TermId;
use crate::control::{AppState, TermState};
use serde_json::{Map, Value, json};

fn number_in(text: &str, prefix: &str) -> Option<u32> {
    let text = text.trim().to_lowercase();
    let rest = text.strip_prefix(prefix).unwrap_or(&text).trim();
    let rest = rest.strip_prefix('#').unwrap_or(rest).trim();
    (!rest.is_empty() && rest.len() <= 10 && rest.bytes().all(|b| b.is_ascii_digit()))
        .then(|| rest.parse().ok())
        .flatten()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn all_terms(app: &AppState) -> impl Iterator<Item = &TermState> {
    app.spaces
        .iter()
        .flat_map(|s| s.tabs.iter())
        .flat_map(|t| t.terminals.iter())
        .chain(app.loose.iter())
}

pub fn resolve_terminal(app: &AppState, value: &Value) -> Result<TermId, String> {
    let text = value_text(value);
    if let Some(n) = number_in(&text, "terminal") {
        return all_terms(app)
            .find(|t| t.number == Some(n))
            .map(|t| t.terminal.clone())
            .ok_or_else(|| format!("There is no terminal {n} in the Nixa window. Read nixa_app_summary for the numbers. Nothing was done."));
    }
    let id: TermId = text.trim().parse().map_err(|_| {
        format!("\"{text}\" is not a terminal number or id. Use one from nixa_app_summary. Nothing was done.")
    })?;
    if !all_terms(app).any(|t| t.terminal == id) {
        return Err(format!("Terminal {id} is not in the Nixa window. Read nixa_app_summary first. Nothing was done."));
    }
    Ok(id)
}

pub fn resolve_space(app: &AppState, value: &Value) -> Result<String, String> {
    let text = value_text(value);
    if let Some(n) = number_in(&text, "space") {
        return app
            .spaces
            .get((n as usize).wrapping_sub(1))
            .map(|s| s.id.clone())
            .ok_or_else(|| format!("There is no space {n}. Read nixa_app_summary for the list. Nothing was done."));
    }
    let wanted = text.trim();
    app.spaces
        .iter()
        .find(|s| s.id == wanted)
        .or_else(|| app.spaces.iter().find(|s| s.name.eq_ignore_ascii_case(wanted)))
        .map(|s| s.id.clone())
        .ok_or_else(|| format!("There is no space \"{wanted}\". Read nixa_app_summary for the list. Nothing was done."))
}

fn type_ok(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        _ => true,
    }
}

pub fn check_args(spec: &Value, args: &Map<String, Value>) -> Result<(), String> {
    let empty = Map::new();
    let properties = spec.get("properties").and_then(Value::as_object).unwrap_or(&empty);
    if spec.get("additionalProperties") == Some(&Value::Bool(false))
        && let Some(key) = args.keys().find(|k| !properties.contains_key(*k))
    {
        return Err(format!("Unknown argument \"{key}\" for this command. Nothing was run."));
    }
    for key in spec.get("required").and_then(Value::as_array).into_iter().flatten() {
        if let Some(key) = key.as_str()
            && !args.contains_key(key)
        {
            return Err(format!("Missing required argument \"{key}\". Nothing was run."));
        }
    }
    for (key, value) in args {
        let Some(rule) = properties.get(key).and_then(Value::as_object) else {
            continue;
        };
        if let Some(allowed) = rule.get("enum").and_then(Value::as_array)
            && !allowed.contains(value)
        {
            return Err(format!("Argument \"{key}\" is not an allowed value. Nothing was run."));
        }
        let kinds: Vec<&str> = match rule.get("type") {
            Some(Value::String(k)) => vec![k.as_str()],
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !kinds.is_empty() && !kinds.iter().any(|k| type_ok(k, value)) {
            return Err(format!("Argument \"{key}\" must be {}. Nothing was run.", kinds.join(" or ")));
        }
    }
    Ok(())
}

pub fn resolve_args(app: &AppState, command: &str, args: &mut Map<String, Value>) -> Result<(), String> {
    if let Some(value) = args.get("terminal").cloned() {
        args.insert("terminal".into(), json!(resolve_terminal(app, &value)?));
    }
    if let Some(value) = args.get("space").cloned() {
        args.insert("space".into(), json!(resolve_space(app, &value)?));
    }
    if command.starts_with("space.")
        && let Some(value) = args.get("id").cloned()
    {
        args.insert("id".into(), json!(resolve_space(app, &value)?));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Status;
    use crate::control::{Focus, SpaceState, TabState};

    fn term(n: u64) -> TermState {
        TermState {
            terminal: TermId::from_number(n),
            number: Some(n as u32),
            name: format!("agent {n}"),
            program: "sh".into(),
            status: Some(Status::Idle),
            note: None,
            running: true,
            children: 0,
            focused: n == 1,
        }
    }

    fn app() -> AppState {
        let space = |id: &str, name: &str, tabs| SpaceState { id: id.into(), name: name.into(), folder: format!("/work/{name}"), active: id == "s1", tabs };
        AppState {
            focus: Focus { space: Some("s1".into()), tab: Some("t1".into()), terminal: Some(TermId::from_number(1)) },
            spaces: vec![
                space("s1", "shop", vec![TabState { id: "t1".into(), name: "agent 1".into(), active: true, terminals: vec![term(1), term(2)] }]),
                space("s7", "notes", Vec::new()),
            ],
            ..AppState::default()
        }
    }

    #[test]
    fn numbers_and_ids_find_terminals_and_spaces() {
        let app = app();
        assert_eq!(resolve_terminal(&app, &json!(2)).unwrap(), TermId::from_number(2));
        assert_eq!(resolve_terminal(&app, &json!("terminal 1")).unwrap(), TermId::from_number(1));
        assert_eq!(resolve_terminal(&app, &json!("m2")).unwrap(), TermId::from_number(2));
        assert!(resolve_terminal(&app, &json!("9")).is_err());
        assert!(resolve_terminal(&app, &json!("m9")).is_err());
        assert!(resolve_terminal(&app, &json!("rm -rf")).is_err());
        assert_eq!(resolve_space(&app, &json!("2")).unwrap(), "s7");
        assert_eq!(resolve_space(&app, &json!("space 1")).unwrap(), "s1");
        assert_eq!(resolve_space(&app, &json!("Notes")).unwrap(), "s7");
        assert!(resolve_space(&app, &json!("0")).is_err());
        assert!(resolve_space(&app, &json!("nope")).is_err());
        let mut args = json!({"id": "2"}).as_object().unwrap().clone();
        resolve_args(&app, "space.select", &mut args).unwrap();
        assert_eq!(args["id"], "s7");
    }

    #[test]
    fn arguments_are_checked_against_the_command() {
        let spec = json!({"type": "object", "properties": {"id": {"type": "string"}, "place": {"enum": ["tab", "right"]}}, "required": ["id"], "additionalProperties": false});
        let args = |v: Value| v.as_object().unwrap().clone();
        assert!(check_args(&spec, &args(json!({"id": "t1", "place": "tab"}))).is_ok());
        assert!(check_args(&spec, &args(json!({}))).unwrap_err().contains("Missing"));
        assert!(check_args(&spec, &args(json!({"id": 3}))).unwrap_err().contains("must be string"));
        assert!(check_args(&spec, &args(json!({"id": "t", "x": 1}))).unwrap_err().contains("Unknown"));
        assert!(check_args(&spec, &args(json!({"id": "t", "place": "up"}))).is_err());
    }
}
