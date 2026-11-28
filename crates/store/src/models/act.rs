use super::pick::{ModelEdit, ModelRequest};
use super::rank::{By, check_capabilities, capabilities_for, model_rows};
use super::usage::{leaderboard, plan_usage};
use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

pub const ACTIONS: [&str; 7] = ["view", "list", "choose", "rank", "move", "edit", "refresh"];
pub const DECISIONS_SHOWN: usize = 10;
pub const MODELS_ABOUT: &str = "The shared model leaderboard and the usage of each plan. view gives one snapshot (ranked rows, secondary chains, counts and what each main capability would pick) and writes nothing; it takes no other arguments. list also gives plan usage and the last picks. Picks follow fixed rules: an explicit model or agent wins, then capability pins, then the saved rank. Capabilities only rank and never block: first a model that meets all needs, then the one that meets most, then any listed model. Saved capabilities join the default tags unless the user set them by hand. When the asked agent has no free model, choose falls over to the next agent in the agent map's fall over order and says which one took it and why. A named model that is not free fails; it is never swapped silently. Every pick is written to the picks log with its rule. rank sorts by public scores, real job results, cost and fit. move puts a row at a new rank (rows the user pinned stay). edit changes tags (capabilities), cost, secondary, disabled (take out), public names, company or plan. refresh fetches public scores now. These local changes run at once without a yes. Nixa may also change pins and capabilities the user set, and each such change is written to the picks log. Never invent model IDs: use agent::model from the table.";
pub const AGENT_MODELS_ABOUT: &str = "The models and efforts one agent can use, from the saved model list (claude has a fixed list). A list older than one day is read again in the background; refresh true reads it from the agent now. Use it before you choose a custom agent's start profile model or effort. The answer also has that agent's leaderboard rows.";

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelArgs {
    pub action: Option<String>,
    pub task: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub named: Option<String>,
    pub agent: Option<String>,
    pub from: Option<String>,
    pub id: Option<String>,
    pub rank: Option<u64>,
    pub secondary: Option<String>,
    pub cost: Option<String>,
    pub disabled: Option<bool>,
    pub pinned: Option<bool>,
    pub pins: Option<Vec<String>>,
    pub public_names: Option<Vec<String>>,
    pub company: Option<String>,
    pub plan: Option<String>,
}

pub fn action_name(args: &ModelArgs) -> &str {
    args.action.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or("list")
}

pub fn parse_args(value: &Value) -> Result<ModelArgs> {
    let value = if value.is_null() { json!({}) } else { value.clone() };
    let args: ModelArgs = serde_json::from_value(value).map_err(|e| anyhow!("Bad arguments: {e}. Nothing was done."))?;
    if !ACTIONS.contains(&action_name(&args)) {
        bail!("Unknown action {}. Use one of: {}.", action_name(&args), ACTIONS.join(", "));
    }
    if action_name(&args) == "view" && args != (ModelArgs { action: args.action.clone(), ..ModelArgs::default() }) {
        bail!("view takes no other arguments.");
    }
    if args.task.as_deref().is_some_and(|t| t.chars().count() > 2000) {
        bail!("task is longer than 2000 characters.");
    }
    if let Some(caps) = &args.capabilities {
        check_capabilities(caps)?;
    }
    if args.rank == Some(0) {
        bail!("rank starts at 1.");
    }
    Ok(args)
}

fn request_of(args: &ModelArgs) -> ModelRequest {
    ModelRequest {
        task: args.task.clone(),
        capabilities: args.capabilities.clone(),
        named: args.named.clone(),
        agent: args.agent.clone(),
        from: args.from.clone(),
        fallover: Vec::new(),
    }
}

fn edit_of(args: &ModelArgs) -> ModelEdit {
    ModelEdit {
        secondary: args.secondary.clone(),
        cost: args.cost.clone(),
        disabled: args.disabled,
        pinned: args.pinned,
        pins: args.pins.clone(),
        public_names: args.public_names.clone(),
        company: args.company.clone(),
        plan: args.plan.clone(),
        capabilities: args.capabilities.clone(),
    }
}

pub fn list_value(conn: &Connection) -> Result<Value> {
    let cache = super::load(conn)?;
    let rows = model_rows(&cache);
    Ok(json!({
        "rows": rows,
        "plans": plan_usage(&rows),
        "decisions": super::decisions(conn, DECISIONS_SHOWN)?,
        "public_refresh": cache.public_refresh,
    }))
}

pub fn view_value(conn: &Connection) -> Result<Value> {
    Ok(serde_json::to_value(leaderboard(&super::read(conn)?))?)
}

pub fn act(conn: &Connection, args: &ModelArgs, by: By) -> Result<Value> {
    match action_name(args) {
        "view" => view_value(conn),
        "choose" => {
            let mut request = request_of(args);
            if request.agent.is_some() {
                request.fallover = crate::agent_map::fallover(conn)?;
            }
            let chosen = super::choose(conn, &request)?;
            let rows = model_rows(&super::load(conn)?);
            Ok(json!({"chosen": chosen, "plans": plan_usage(&rows)}))
        }
        "rank" => {
            let needs = match (&args.capabilities, &args.task) {
                (Some(caps), _) => caps.clone(),
                (None, Some(task)) => capabilities_for(task),
                (None, None) => Vec::new(),
            };
            Ok(json!({"rows": model_rows(&super::rank(conn, &needs)?)}))
        }
        "move" => {
            let (Some(id), Some(rank)) = (args.id.as_deref(), args.rank) else {
                bail!("Give a model ID and target rank.");
            };
            Ok(json!({"rows": model_rows(&super::move_to(conn, id, rank as usize, by)?)}))
        }
        "edit" => {
            let Some(id) = args.id.as_deref() else {
                bail!("Give a model ID.");
            };
            Ok(json!({"rows": model_rows(&super::edit(conn, id, &edit_of(args), by)?)}))
        }
        "refresh" => bail!("Public scores are fetched by the brain."),
        _ => list_value(conn),
    }
}
