use serde::Deserialize;
use serde_json::{Value, json};

pub const CAPS: [&str; 10] = [
    "planning",
    "coding",
    "refactor",
    "ui",
    "testing",
    "cheap",
    "big-context",
    "vision",
    "long-reasoning",
    "speed",
];
pub const COSTS: [&str; 3] = ["cheap", "standard", "premium"];

pub const TABLE_COLUMNS: [(&str, f32); 9] = [
    ("", 26.0),
    ("Rank", 42.0),
    ("Agent · model", 230.0),
    ("Public / ours", 135.0),
    ("Capabilities", 275.0),
    ("Cost", 76.0),
    ("State", 85.0),
    ("Secondary", 190.0),
    ("Moved by", 60.0),
];
pub const TABLE_WIDTH: f32 = 1300.0;

pub const PLAN_COLUMNS: [(&str, f32); 5] = [
    ("Plan", 180.0),
    ("Jobs", 55.0),
    ("Good / bad", 90.0),
    ("Available", 120.0),
    ("Last used", 185.0),
];

pub const BOARD_COLUMNS: [(&str, f32); 14] = [
    ("Rank", 50.0),
    ("Agent · model", 260.0),
    ("Company", 130.0),
    ("Plan", 170.0),
    ("Cost", 90.0),
    ("Capabilities", 300.0),
    ("Public score", 100.0),
    ("Nixa score", 100.0),
    ("Good / bad", 100.0),
    ("Secondary chain", 430.0),
    ("Pinned", 95.0),
    ("Moved by", 95.0),
    ("Moved at", 220.0),
    ("Availability", 140.0),
];

pub const EDIT_FIELDS: [(&str, Field); 4] = [
    ("Capabilities (comma separated)", Field::Capabilities),
    ("Secondary (agent::model)", Field::Secondary),
    ("Pin for capabilities (comma separated)", Field::Pins),
    (
        "Exact public model names (comma separated)",
        Field::PublicNames,
    ),
];

pub const HINT_TEXT: &str =
    "Move rows to rank them. A hand move pins the row; Nixa keeps pinned places.";
pub const CONNECTING_TEXT: &str = "Connecting to Nixa…";
pub const EMPTY_TEXT: &str = "No models yet. List an agent's models to fill the model table.";
pub const FOOT_TEXT: &str = "The chooser reads the same model table.";
pub const BOARD_EMPTY_TEXT: &str =
    "No model rows received. This view never fills or refreshes the model table.";
pub const DRAG_TIP: &str = "Drag to another row to set its rank and pin it";
pub const SAVING_TEXT: &str = "Saving…";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Capabilities,
    Secondary,
    Pins,
    PublicNames,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Row {
    pub id: String,
    pub agent: String,
    pub model: String,
    pub rank: usize,
    pub company: String,
    pub plan: String,
    pub cost: String,
    pub efforts: Vec<String>,
    pub capabilities: Vec<String>,
    #[serde(alias = "publicScore")]
    pub public_score: Option<f64>,
    #[serde(alias = "publicSource")]
    pub public_source: Option<String>,
    #[serde(alias = "publicNames")]
    pub public_names: Vec<String>,
    #[serde(alias = "ownScore")]
    pub own_score: Option<f64>,
    pub good: u64,
    pub bad: u64,
    pub secondary: Option<String>,
    #[serde(alias = "secondaryChain")]
    pub secondary_chain: Vec<String>,
    pub pins: Vec<String>,
    pub pinned: bool,
    pub disabled: bool,
    pub listed: Option<bool>,
    #[serde(alias = "movedBy")]
    pub moved_by: Option<String>,
    #[serde(alias = "movedAt")]
    pub moved_at: Value,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Plan {
    pub plan: String,
    pub companies: Vec<String>,
    pub included: bool,
    pub jobs: u64,
    pub good: u64,
    pub bad: u64,
    pub available: Value,
    #[serde(alias = "lastAt")]
    pub last_at: Value,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Decision {
    pub model: Option<String>,
    pub rule: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Chosen {
    pub id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Choice {
    pub capability: String,
    pub chosen: Option<Chosen>,
    pub rule: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Refresh {
    pub at: Value,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Table {
    pub rows: Vec<Row>,
    pub plans: Vec<Plan>,
    pub decisions: Vec<Decision>,
    pub file: Option<String>,
    #[serde(alias = "scoresOnline")]
    pub scores_online: Option<bool>,
    #[serde(alias = "scoresNote")]
    pub scores_note: String,
    #[serde(alias = "publicRefresh")]
    pub public_refresh: Option<Refresh>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Board {
    pub rows: Vec<Row>,
    pub choices: Vec<Choice>,
    pub listed: usize,
    pub pinned: usize,
    pub file: Option<String>,
    #[serde(alias = "scoresOnline")]
    pub scores_online: Option<bool>,
    #[serde(alias = "scoresNote")]
    pub scores_note: String,
}

pub fn data(value: &Value) -> &Value {
    match value.get("data") {
        Some(inner) if inner.is_object() => inner,
        _ => value,
    }
}

pub fn rows_of(value: Option<&Value>) -> Option<Vec<Row>> {
    let list = value?.as_array()?;
    Some(
        list.iter()
            .filter_map(|v| serde_json::from_value::<Row>(v.clone()).ok())
            .filter(|r| !r.id.is_empty())
            .collect(),
    )
}

fn list_of<T: serde::de::DeserializeOwned>(value: Option<&Value>) -> Vec<T> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn text_of(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

pub fn parse_table(value: &Value) -> Table {
    let data = data(value);
    Table {
        rows: rows_of(data.get("rows")).unwrap_or_default(),
        plans: list_of(data.get("plans")),
        decisions: list_of(data.get("decisions")),
        file: text_of(data.get("file")),
        scores_online: data.get("scoresOnline").and_then(Value::as_bool),
        scores_note: text_of(data.get("scoresNote")).unwrap_or_default(),
        public_refresh: data
            .get("public_refresh")
            .or_else(|| data.get("publicRefresh"))
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
    }
}

pub fn parse_board(value: &Value) -> Board {
    let data = data(value);
    let count = |key: &str| data.get(key).and_then(Value::as_u64).unwrap_or(0) as usize;
    Board {
        rows: rows_of(data.get("rows")).unwrap_or_default(),
        choices: list_of(data.get("choices")),
        listed: count("listed"),
        pinned: count("pinned"),
        file: text_of(data.get("file")),
        scores_online: data.get("scoresOnline").and_then(Value::as_bool),
        scores_note: text_of(data.get("scoresNote")).unwrap_or_default(),
    }
}

pub fn reply_rows(value: &Value) -> Option<Vec<Row>> {
    rows_of(data(value).get("rows"))
}

pub fn time_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => {
            let ms = n.as_i64().filter(|ms| *ms > 0)?;
            let days = ms.div_euclid(86_400_000);
            let rest = ms.rem_euclid(86_400_000) / 60_000;
            let (y, mo, d) = nixa_store::time::civil_from_days(days);
            Some(format!(
                "{y:04}-{mo:02}-{d:02} {:02}:{:02} UTC",
                rest / 60,
                rest % 60
            ))
        }
        _ => None,
    }
}

fn filled(text: &Option<String>) -> Option<&str> {
    text.as_deref().filter(|s| !s.trim().is_empty())
}

pub fn score(n: Option<f64>) -> String {
    match n {
        Some(n) if n.is_finite() => format!("{n:.1}"),
        _ => "unknown".into(),
    }
}

pub fn name(row: &Row) -> String {
    format!("{} · {}", row.agent, row.model)
}

pub fn state_text(row: &Row) -> &'static str {
    if row.listed == Some(false) {
        "unlisted"
    } else if row.disabled {
        "taken out"
    } else {
        "available"
    }
}

pub fn board_state_text(row: &Row) -> &'static str {
    if row.listed == Some(false) {
        "unlisted"
    } else if row.disabled {
        "disabled"
    } else {
        "available"
    }
}

pub fn name_tip(row: &Row) -> String {
    format!(
        "{} · {}\nEfforts: {}",
        row.company,
        row.plan,
        row.efforts.join(", ")
    )
}

pub fn scores(row: &Row) -> String {
    format!("{} / {}", score(row.public_score), score(row.own_score))
}

pub fn scores_tip(row: &Row) -> String {
    format!(
        "{}\nOur results: {} good, {} bad",
        filled(&row.public_source).unwrap_or("No public match"),
        row.good,
        row.bad
    )
}

pub fn secondary_text(row: &Row, rows: &[Row]) -> String {
    row.secondary
        .as_deref()
        .and_then(|id| rows.iter().find(|r| r.id == id))
        .map(name)
        .unwrap_or_else(|| "none".into())
}

pub fn secondary_tip(row: &Row) -> String {
    row.secondary
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "No secondary".into())
}

pub fn moved_text(row: &Row) -> String {
    filled(&row.moved_by).unwrap_or("—").to_string()
}

pub fn moved_tip(row: &Row) -> String {
    time_text(&row.moved_at).unwrap_or_else(|| "Initial order".into())
}

pub fn shown<'a>(rows: &'a [Row], filter: &str) -> Vec<&'a Row> {
    rows.iter()
        .filter(|r| filter.is_empty() || r.capabilities.iter().any(|c| c == filter))
        .collect()
}

pub fn up_rank(row: &Row) -> usize {
    row.rank.saturating_sub(1).max(1)
}

pub fn down_rank(row: &Row, count: usize) -> usize {
    (row.rank + 1).min(count.max(1))
}

pub fn move_body(id: &str, rank: usize) -> Value {
    json!({"id": id, "rank": rank})
}

pub fn drop_body(dragged: Option<&str>, target: &Row) -> Option<Value> {
    let dragged = dragged?;
    (dragged != target.id).then(|| move_body(dragged, target.rank))
}

pub fn pin_body(row: &Row) -> Value {
    json!({"id": row.id, "pinned": !row.pinned})
}

pub fn out_body(row: &Row) -> Value {
    json!({"id": row.id, "disabled": !row.disabled})
}

pub fn cost_body(row: &Row, cost: &str) -> Value {
    json!({"id": row.id, "cost": cost})
}

pub fn rank_body(filter: &str) -> Value {
    let caps: Vec<&str> = if filter.is_empty() {
        vec![]
    } else {
        vec![filter]
    };
    json!({"capabilities": caps})
}

pub fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn field_text(row: &Row, field: Field) -> String {
    match field {
        Field::Capabilities => row.capabilities.join(", "),
        Field::Secondary => row.secondary.clone().unwrap_or_default(),
        Field::Pins => row.pins.join(", "),
        Field::PublicNames => row.public_names.join(", "),
    }
}

pub fn fields_body(id: &str, caps: &str, secondary: &str, pins: &str, public_names: &str) -> Value {
    json!({
        "id": id,
        "capabilities": split_list(caps),
        "secondary": secondary.trim(),
        "pins": split_list(pins),
        "public_names": split_list(public_names),
    })
}

pub fn scores_status(online: Option<bool>, note: &str) -> Option<&str> {
    if !note.is_empty() {
        Some(note)
    } else {
        online.map(|on| {
            if on {
                "Public scores are on."
            } else {
                "Public scores are off."
            }
        })
    }
}

pub fn info_text(busy: bool, table: &Table) -> String {
    if busy {
        return SAVING_TEXT.into();
    }
    let at = table
        .public_refresh
        .as_ref()
        .and_then(|r| time_text(&r.at))
        .unwrap_or_else(|| "not yet".into());
    format!("Public / our score (0–100). Unknown scores stay unknown. Public refresh: {at}")
}

pub fn plan_tip(plan: &Plan) -> String {
    let how = if plan.included {
        "Included in plan"
    } else {
        "Paid apart"
    };
    format!("{} · {how}", plan.companies.join(", "))
}

pub fn plan_cells(plan: &Plan) -> [String; 5] {
    let available = match &plan.available {
        Value::Null => "—".to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    [
        plan.plan.clone(),
        plan.jobs.to_string(),
        format!("{} / {}", plan.good, plan.bad),
        available,
        time_text(&plan.last_at).unwrap_or_else(|| "Not used yet".into()),
    ]
}

pub fn decision_lines(decisions: &[Decision]) -> Vec<String> {
    let start = decisions.len().saturating_sub(5);
    decisions[start..]
        .iter()
        .rev()
        .map(|d| {
            format!(
                "{} · {}",
                d.model.as_deref().unwrap_or("No model available"),
                d.rule
            )
        })
        .collect()
}

pub fn board_title(board: &Board) -> String {
    format!(
        "Model leaderboard · {} listed · {} pinned · {} rows · Read only",
        board.listed,
        board.pinned,
        board.rows.len()
    )
}

pub fn choice_text(choice: &Choice) -> String {
    let chosen = choice
        .chosen
        .as_ref()
        .map(|c| c.id.clone())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "No available model".into());
    format!("{}: {chosen}", choice.capability)
}

pub fn board_cells(row: &Row) -> [String; 14] {
    let chain = if row.secondary_chain.is_empty() {
        "—".to_string()
    } else {
        row.secondary_chain.join(" → ")
    };
    [
        row.rank.to_string(),
        name(row),
        row.company.clone(),
        row.plan.clone(),
        row.cost.clone(),
        row.capabilities.join(", "),
        score(row.public_score),
        score(row.own_score),
        format!("{} / {}", row.good, row.bad),
        chain,
        if row.pinned { "★ Pinned" } else { "—" }.into(),
        moved_text(row),
        time_text(&row.moved_at).unwrap_or_else(|| "—".into()),
        board_state_text(row).into(),
    ]
}

pub fn board_width() -> f32 {
    BOARD_COLUMNS.iter().map(|c| c.1).sum()
}

pub fn failed_text(error: &str) -> String {
    format!("Last read failed: {error}. Showing last received snapshot.")
}
