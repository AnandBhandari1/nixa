use crate::app::{App, lock};
use crate::util::log;
use nixa_protocol::control::{self, ControlRequest};
use nixa_protocol::id::TermId;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub const FOCUS_WAIT: Duration = Duration::from_secs(10);
pub const ANSWERED: &str = "That question is already answered.";

pub fn agent_word(agent: &str) -> String {
    match agent.trim() {
        "" => "the agent".into(),
        name => name.to_string(),
    }
}

pub async fn show_on(app: &App, message: &Value, socket: PathBuf) -> Result<Value, String> {
    let id = message
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or("show needs the id of an agent question.")?;
    let found = lock(&app.prompts.book).get(id).map(|p| (p.id.clone(), p.terminal.clone(), p.agent.clone()));
    let Some((id, terminal, agent)) = found else {
        app.info(ANSWERED);
        return Err(ANSWERED.into());
    };
    let shown = match terminal.parse::<TermId>() {
        Ok(term) => focus(socket, term).await,
        Err(_) => Err(format!("{terminal} is not a terminal id")),
    };
    match shown {
        Ok(()) => {
            log("show", &format!("shown {id} in {terminal}"));
            Ok(json!({"id": id, "terminal": terminal, "shown": true}))
        }
        Err(error) => {
            log("show", &format!("could not show {id} in {terminal}: {error}"));
            let line = format!("Could not show {}'s terminal.", agent_word(&agent));
            app.info(&line);
            Err(format!("{line} {error}"))
        }
    }
}

async fn focus(socket: PathBuf, terminal: TermId) -> Result<(), String> {
    let request = ControlRequest::Focus { terminal };
    tokio::task::spawn_blocking(move || control::call(&socket, &request, FOCUS_WAIT))
        .await
        .map_err(|error| error.to_string())?
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}
