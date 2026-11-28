use crate::actions::{Action, Body, Kind, inputs};
use serde_json::{Value, json};

fn names(about: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": about})
}

fn step(name: &'static str) -> Value {
    json!({"type": "string", "enum": [name], "description": format!("Must be {name}.")})
}

fn agent_fields() -> Value {
    json!({
        "title": {"type": "string", "description": "Short name of the custom agent."},
        "purpose": {"type": "string", "description": "The job this agent does."},
        "instructions": {"type": "string", "description": "Standing instructions for the agent."},
        "role": {"type": "string", "description": "Role, for example reviewer. Empty clears it."},
        "job": {"type": "string", "description": "Job tag. Empty clears it."},
        "workflow": {"type": "string", "description": "Default workflow id. Empty clears it."},
        "session": {"type": "string", "description": "Session id to resume."},
        "agent": {"type": "string", "description": "Start profile agent, for example claude or codex."},
        "model": {"type": "string", "description": "Start profile model."},
        "effort": {"type": "string", "description": "Start profile effort."},
        "off": {"type": "string", "description": "on or off. off takes the agent out of use."}
    })
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(map), Some(more)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in more {
            map.insert(key.clone(), value.clone());
        }
    }
    base
}

pub fn list() -> Vec<Action> {
    let mut out = teams();
    out.extend(custom_agents());
    out.extend(models());
    out
}

fn teams() -> Vec<Action> {
    vec![
        Action {
            name: "team.list",
            method: "GET",
            path: "/teams",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the teams, their roles, the custom agents and the last team runs.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "team.save",
            method: "POST",
            path: "/teams",
            body: Body::Payload("save"),
            kind: Kind::Write,
            help: "Add a team, or change one when its id is given.",
            inputs: inputs(
                json!({
                    "team": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Team id. Small letters, numbers and dashes. Leave out to make one from the name."},
                            "name": {"type": "string", "description": "Team name, at most 60 characters."},
                            "purpose": {"type": "string", "description": "What the team is for."},
                            "steps": {
                                "type": "array",
                                "description": "The steps in order. Each step names a custom agent id.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "agent": {"type": "string", "description": "Custom agent id."},
                                        "note": {"type": "string", "description": "Note for this step."}
                                    },
                                    "required": ["agent"]
                                }
                            },
                            "skills": names("Skill names the team uses."),
                            "mcps": names("MCP server names the team uses."),
                            "flow": {"type": "string", "enum": ["task", "pr"], "description": "Default task."}
                        },
                        "required": ["name", "steps"]
                    }
                }),
                &["team"],
            ),
            card: Some("Save the team \"{team.name}\"?"),
        },
        Action {
            name: "team.remove",
            method: "POST",
            path: "/teams",
            body: Body::Payload("remove"),
            kind: Kind::Write,
            help: "Delete a team. A team with a running run cannot be deleted.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Team id from team.list."}}), &["id"]),
            card: Some("Delete the team {id}?"),
        },
        Action {
            name: "team.start",
            method: "POST",
            path: "/teams",
            body: Body::Payload("start"),
            kind: Kind::Write,
            help: "Start a team run on a task in a space or a project folder.",
            inputs: inputs(
                json!({
                    "team": {"type": "string", "description": "Team id from team.list."},
                    "task": {"type": "string", "description": "The task for the team."},
                    "space": {"type": "string", "description": "Space id or name. Give this or project."},
                    "project": {"type": "string", "description": "Project folder path. Give this or space."},
                    "repo": {"type": "string", "description": "Repo path or link."},
                    "branch": {"type": "string", "description": "Branch to work on."},
                    "skills": names("Extra skill names for this run."),
                    "mcps": names("Extra MCP server names for this run.")
                }),
                &["team", "task"],
            ),
            card: Some("Start the team {team} on \"{task}\" in {space}{project}?"),
        },
        Action {
            name: "team.stop",
            method: "POST",
            path: "/teams",
            body: Body::Payload("stop"),
            kind: Kind::Write,
            help: "Stop a team run.",
            inputs: inputs(json!({"run": {"type": "string", "description": "Run id from team.list."}}), &["run"]),
            card: Some("Stop the team run {run}?"),
        },
        Action {
            name: "team.resume",
            method: "POST",
            path: "/teams",
            body: Body::Payload("resume"),
            kind: Kind::Write,
            help: "Resume a stopped team run.",
            inputs: inputs(json!({"run": {"type": "string", "description": "Run id from team.list."}}), &["run"]),
            card: Some("Resume the team run {run}?"),
        },
    ]
}

fn custom_agents() -> Vec<Action> {
    vec![
        Action {
            name: "agent.list",
            method: "GET",
            path: "/custom-agents",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the saved custom agents, which are free, the spaces, teams and workflows.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "agent.find",
            method: "POST",
            path: "/custom-agents/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Find saved custom agents that fit a purpose.",
            inputs: inputs(
                json!({
                    "action": step("find"),
                    "query": {"type": "string", "description": "The purpose to look for."},
                    "space": {"type": "string", "description": "Only agents in this space id or name."}
                }),
                &["action", "query"],
            ),
            card: None,
        },
        Action {
            name: "agent.save",
            method: "POST",
            path: "/custom-agents/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Save the agent running in a terminal as a custom agent, or update the saved one.",
            inputs: inputs(
                with(
                    agent_fields(),
                    json!({
                        "action": step("save"),
                        "terminal": {"type": "string", "description": "Terminal id where the agent runs."},
                        "provider": {"type": "string", "description": "Agent provider. Default is the one running in the terminal."}
                    }),
                ),
                &["action", "terminal"],
            ),
            card: Some("Save the agent in terminal {terminal} as a custom agent \"{title}\"?"),
        },
        Action {
            name: "agent.add",
            method: "POST",
            path: "/custom-agents/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Add a new custom agent with a title and a purpose.",
            inputs: inputs(
                with(
                    agent_fields(),
                    json!({
                        "action": step("add"),
                        "provider": {"type": "string", "description": "Agent provider when there is no start profile."},
                        "space": {"type": "string", "description": "Space id or name for the agent."}
                    }),
                ),
                &["action", "title", "purpose"],
            ),
            card: Some("Add the custom agent \"{title}\" for \"{purpose}\"?"),
        },
        Action {
            name: "agent.edit",
            method: "POST",
            path: "/custom-agents/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Change a saved custom agent. Only the given fields change.",
            inputs: inputs(
                with(
                    agent_fields(),
                    json!({
                        "action": step("edit"),
                        "id": {"type": "string", "description": "Custom agent id from agent.list."}
                    }),
                ),
                &["action", "id"],
            ),
            card: Some("Change the custom agent {id}?"),
        },
        Action {
            name: "agent.use",
            method: "POST",
            path: "/custom-agents/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Use a custom agent: bring it back or start it, and send it a message.",
            inputs: inputs(
                json!({
                    "action": step("use"),
                    "id": {"type": "string", "description": "Custom agent id. Give this or query."},
                    "query": {"type": "string", "description": "A title or a purpose to find the agent."},
                    "message": {"type": "string", "description": "Message to send to the agent."},
                    "space": {"type": "string", "description": "Space id or name to start it in."},
                    "show": {"type": "boolean", "description": "true shows its terminal."},
                    "copy": {"type": "boolean", "description": "true starts a new copy when it is busy."}
                }),
                &["action"],
            ),
            card: Some("Bring back or start the custom agent {id}{query} (new copy: {copy}) and send: {message}?"),
        },
        Action {
            name: "agent.forget",
            method: "POST",
            path: "/custom-agents/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Forget a saved custom agent. Its terminal stays open.",
            inputs: inputs(
                json!({
                    "action": step("forget"),
                    "id": {"type": "string", "description": "Custom agent id from agent.list."}
                }),
                &["action", "id"],
            ),
            card: Some("Forget the custom agent {id}?"),
        },
    ]
}

fn models() -> Vec<Action> {
    vec![
        Action {
            name: "model.list",
            method: "GET",
            path: "/models",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the model leaderboard rows, plan usage and the last picks.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "model.view",
            method: "GET",
            path: "/models/view",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show one snapshot of the model leaderboard and what each capability would pick.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "model.choose",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Pick a model for a task. The pick is written to the picks log.",
            inputs: inputs(
                json!({
                    "action": step("choose"),
                    "task": {"type": "string", "description": "The task, at most 2000 characters."},
                    "capabilities": names("Capabilities the task needs."),
                    "named": {"type": "string", "description": "An exact model to use."},
                    "agent": {"type": "string", "description": "Agent to pick a model for."},
                    "from": {"type": "string", "description": "Who asks for the pick."}
                }),
                &["action"],
            ),
            card: Some("Pick a model for \"{task}\" {agent} {named}?"),
        },
        Action {
            name: "model.rank",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Rank the models by public scores, job results, cost and fit.",
            inputs: inputs(
                json!({
                    "action": step("rank"),
                    "task": {"type": "string", "description": "A task to rank for."},
                    "capabilities": names("Capabilities to rank for.")
                }),
                &["action"],
            ),
            card: Some("Rank the models again for \"{task}\"?"),
        },
        Action {
            name: "model.move",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Move a model row to a new rank.",
            inputs: inputs(
                json!({
                    "action": step("move"),
                    "id": {"type": "string", "description": "Model id, agent::model, from model.list."},
                    "rank": {"type": "integer", "minimum": 1, "description": "The new rank, from 1."}
                }),
                &["action", "id", "rank"],
            ),
            card: Some("Move the model {id} to rank {rank}?"),
        },
        Action {
            name: "model.edit",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Change a model row: tags, cost, secondary, disabled, pins, public names, company or plan.",
            inputs: inputs(
                json!({
                    "action": step("edit"),
                    "id": {"type": "string", "description": "Model id, agent::model, from model.list."},
                    "secondary": {"type": "string", "description": "The secondary model id."},
                    "cost": {"type": "string", "description": "Cost note."},
                    "disabled": {"type": "boolean", "description": "true takes the model out."},
                    "pinned": {"type": "boolean", "description": "true pins its rank."},
                    "pins": names("Capabilities this model is pinned for."),
                    "public_names": names("Names of this model on public leaderboards."),
                    "company": {"type": "string", "description": "Company that makes the model."},
                    "plan": {"type": "string", "description": "Plan the model uses."},
                    "capabilities": names("Capability tags of the model.")
                }),
                &["action", "id"],
            ),
            card: Some("Change the model {id}?"),
        },
        Action {
            name: "model.refresh",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Read the public model scores now. Works only when online model scores are on (model.online).",
            inputs: inputs(json!({"action": step("refresh")}), &["action"]),
            card: Some("Read the public model scores from the internet now?"),
        },
        Action {
            name: "model.agent_models",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the models and efforts one agent can use, with its leaderboard rows.",
            inputs: inputs(
                json!({
                    "action": step("agent_models"),
                    "agent": {"type": "string", "description": "Agent name, for example claude or codex."}
                }),
                &["action", "agent"],
            ),
            card: None,
        },
        Action {
            name: "model.agent_models_refresh",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Read the model list from one agent now and save it.",
            inputs: inputs(
                json!({
                    "action": step("agent_models"),
                    "agent": {"type": "string", "description": "Agent name, for example claude or codex."},
                    "refresh": {"type": "boolean", "enum": [true], "description": "Must be true."}
                }),
                &["action", "agent", "refresh"],
            ),
            card: Some("Read the model list from agent {agent} now and save it?"),
        },
        Action {
            name: "model.refresh_lists",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Read the model list of every installed agent again now, in the background, and save it in the shared model cache.",
            inputs: inputs(json!({"action": step("refresh_lists")}), &["action"]),
            card: Some("Read the model lists of all installed agents again now?"),
        },
        Action {
            name: "model.online",
            method: "POST",
            path: "/models/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Turn reading public model scores from the internet on or off.",
            inputs: inputs(
                json!({
                    "action": step("online"),
                    "on": {"type": "boolean", "description": "true turns it on, false turns it off."}
                }),
                &["action", "on"],
            ),
            card: Some("Turn public model scores from the internet on: {on}?"),
        },
    ]
}
