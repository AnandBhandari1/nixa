use super::actions::{ACTIONS, Who, action};
use crate::app::App;
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::turns::ToolSpec;
use nixa_store::agents::{PROFILE_AGENTS, REVIEWER_ROLE};
use serde_json::json;
use std::sync::Arc;

pub const NAME: &str = "nixa_custom_agents";

const ABOUT: &str = "Custom agents are agents of the left sidebar with a name tag (title), a purpose and optional standing instructions, or entries written by hand with add. This list keeps their space and session so they can be reused or resumed. States: free, working, waiting, closed. list and find combine the terminal status with Nixa's own records: task claims held in the terminal in any space (also tasks with no space), watched jobs, screen questions and deep brain seats. busy lists each reason with its task number, free lists only the agents that are free now, and unsure says when the evidence is incomplete, such as a terminal that reports no status. use checks again just before it sends. It is not the agent map, which picks a model for new work. Before you start a new agent for a job, call find with the purpose. When one fits, call use with its id and a message: Nixa reuses it when it is open and free, resumes its saved session in a new tab of its own space when it is closed, starts a new session when it has none yet, and says so when it is busy, so pick another free agent then or one with a start profile. An agent with a start profile (agent, model, effort) gets a fresh tab from its profile when it is busy. When Nixa cannot tell if it is free, use looks again once after a short wait; if it still cannot tell, it starts a fresh tab from the start profile, or sends the message anyway when the agent has none. Off agents are not picked. Nixa sends the saved instructions before each message by itself. save keeps the agent of a terminal (terminal, title, purpose). add defines one by hand (title, purpose, provider, and optional space, session, instructions, agent, model, effort). edit changes title, purpose, instructions (empty clears them), session, role, job, workflow, agent, model, effort or off (on or off). forget removes an entry; its terminal stays open. show true also shows the terminal on the screen. Role reviewer makes an agent the review agent of its space: finished code work there is passed to it once; it only reads and reports FINDINGS: <number>. A reviewer never runs on Claude. A message sent with use starts a watched job, and Nixa links it to a task: the one the message names, or a new one.";

pub fn specs() -> Vec<ToolSpec> {
    let props = json!({
        "action": {"type": "string", "enum": ACTIONS},
        "id": {"type": "string", "maxLength": 80},
        "query": {"type": "string", "maxLength": 200},
        "terminal": {"type": "string", "maxLength": 20},
        "title": {"type": "string", "maxLength": 60},
        "purpose": {"type": "string", "maxLength": 200},
        "instructions": {"type": "string", "maxLength": 2000},
        "skills": {"type": "string", "description": "Comma list of up to 8 skill names from <data>/skills/NAME/SKILL.md; the agent reads them at start. Empty clears."},
        "session": {"type": "string", "maxLength": 512},
        "provider": {"type": "string", "maxLength": 30},
        "role": {"type": "string", "enum": [REVIEWER_ROLE, "none"]},
        "job": {"type": "string", "maxLength": 40},
        "workflow": {"type": "string", "maxLength": 60},
        "agent": {"type": "string", "enum": PROFILE_AGENTS},
        "model": {"type": "string", "maxLength": 120},
        "effort": {"type": "string", "maxLength": 20},
        "off": {"type": "string", "enum": ["on", "off"]},
        "space": {"type": "string", "maxLength": 120},
        "message": {"type": "string", "maxLength": 8000},
        "show": {"type": "boolean"}
    });
    vec![spec(NAME, ABOUT, schema(props, &["action"]))]
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let name = text(args, "action").unwrap_or_default();
    let who = Who {
        name: format!("{}'s brain", app.name()),
        by_user: false,
        caller: None,
    };
    let out = action(app, &name, args, &who).await?;
    let mut linked = String::new();
    if name == "use"
        && out.data.get("ok").and_then(|v| v.as_bool()) == Some(true)
        && let Some(message) = text(args, "message")
        && let Some(terminal) = out.data.get("terminal").and_then(|v| v.as_str())
    {
        let title = out.data.get("agent").and_then(|a| a.get("title")).and_then(|v| v.as_str()).unwrap_or_default();
        if let Some(task) = super::job::watch_use(app, terminal, &message, title).await {
            linked = format!(" Task #{task} tracks this job.");
        }
    }
    if name == "use"
        && out.data.get("show").and_then(|v| v.as_bool()) == Some(true)
        && let Some(terminal) = out.data.get("terminal").and_then(|v| v.as_str())
    {
        app.send_out(json!({"type": "custom_agents_show", "terminal": terminal}));
    }
    Ok(format!("{}{linked}", out.text))
}
