use crate::actions::{Action, Body, Kind, inputs};
use serde_json::json;

pub fn list() -> Vec<Action> {
    vec![
        Action {
            name: "schedule.list",
            method: "GET",
            path: "/schedules",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the schedules, the repeat messages and the time zone.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "repeat.save",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("repeat.save"),
            kind: Kind::Write,
            help: "Add a repeat message, or change one when id is given.",
            inputs: inputs(
                json!({
                    "repeat": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Repeat id to change. Leave out to add a new one."},
                            "text": {"type": "string", "description": "The message Nixa chat gets."},
                            "every": {"type": "integer", "minimum": 1, "description": "How often, a whole number."},
                            "unit": {"type": "string", "enum": ["minutes", "hours"], "description": "Default minutes."},
                            "enabled": {"type": "boolean", "description": "Default true."}
                        },
                        "required": ["text", "every"]
                    }
                }),
                &["repeat"],
            ),
            card: Some("Save the repeat message \"{repeat.text}\" every {repeat.every} {repeat.unit}?"),
        },
        Action {
            name: "repeat.toggle",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("repeat.toggle"),
            kind: Kind::Write,
            help: "Turn a repeat message on or off.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Repeat id from schedule.list."},
                    "enabled": {"type": "boolean", "description": "true turns it on, false turns it off."}
                }),
                &["id", "enabled"],
            ),
            card: Some("Turn the repeat message {id} on: {enabled}?"),
        },
        Action {
            name: "repeat.send",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("repeat.send"),
            kind: Kind::Write,
            help: "Send a repeat message to Nixa chat now.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Repeat id from schedule.list."}}), &["id"]),
            card: Some("Send the repeat message {id} now?"),
        },
        Action {
            name: "repeat.delete",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("repeat.delete"),
            kind: Kind::Write,
            help: "Delete a repeat message.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Repeat id from schedule.list."}}), &["id"]),
            card: Some("Delete the repeat message {id}?"),
        },
        Action {
            name: "scheduler.set",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("scheduler.set"),
            kind: Kind::Write,
            help: "Turn all schedules on or off at once.",
            inputs: inputs(json!({"enabled": {"type": "boolean", "description": "true turns the scheduler on, false turns it off."}}), &["enabled"]),
            card: Some("Turn the scheduler on: {enabled}?"),
        },
        Action {
            name: "schedule.save",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("schedule.save"),
            kind: Kind::Write,
            help: "Add a schedule, or change one when id is given.",
            inputs: inputs(
                json!({
                    "schedule": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Schedule id to change. Leave out to add a new one."},
                            "name": {"type": "string", "description": "Schedule name."},
                            "agent": {"type": "string", "description": "Agent that runs it, or a team."},
                            "folder": {"type": "string", "description": "Full folder path like ~/project, or a space name."},
                            "prompt": {"type": "string", "description": "Instructions. Needed unless the agent is a team."},
                            "repeat": {"type": "string", "enum": ["once", "hourly", "daily", "weekdays", "weekly"]},
                            "time": {"type": "string", "description": "24-hour time like 09:30."},
                            "date": {"type": "string", "description": "Date like 2026-11-09. Needed when repeat is once."},
                            "days": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 6}, "description": "Week days 0 to 6. Needed when repeat is weekly."},
                            "enabled": {"type": "boolean", "description": "Default false."},
                            "follow_agent": {"type": "string", "description": "Agent that runs after it. Not for a team."},
                            "follow_prompt": {"type": "string", "description": "Instructions for the follow agent."},
                            "skip_unchanged": {"type": "boolean", "description": "Skip a run when nothing changed."},
                            "job": {
                                "type": "object",
                                "description": "Team job details. Only for a team agent.",
                                "properties": {
                                    "space": {"type": "string"},
                                    "repo": {"type": "string"},
                                    "branch": {"type": "string"},
                                    "skills": {"type": "array", "items": {"type": "string"}},
                                    "mcps": {"type": "array", "items": {"type": "string"}}
                                }
                            }
                        },
                        "required": ["name", "agent", "folder", "repeat", "time"]
                    }
                }),
                &["schedule"],
            ),
            card: Some("Save the schedule \"{schedule.name}\" for {schedule.agent}, {schedule.repeat} at {schedule.time}?"),
        },
        Action {
            name: "schedule.toggle",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("schedule.toggle"),
            kind: Kind::Write,
            help: "Turn a schedule on or off.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Schedule id from schedule.list."},
                    "enabled": {"type": "boolean", "description": "true turns it on, false turns it off."}
                }),
                &["id", "enabled"],
            ),
            card: Some("Turn the schedule {id} on: {enabled}?"),
        },
        Action {
            name: "schedule.skip",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("schedule.skip"),
            kind: Kind::Write,
            help: "Skip the next run of a schedule.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Schedule id from schedule.list."}}), &["id"]),
            card: Some("Skip the next run of the schedule {id}?"),
        },
        Action {
            name: "schedule.delete",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("schedule.delete"),
            kind: Kind::Write,
            help: "Delete a schedule.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Schedule id from schedule.list."}}), &["id"]),
            card: Some("Delete the schedule {id}?"),
        },
        Action {
            name: "schedule.run",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("schedule.run"),
            kind: Kind::Write,
            help: "Run a schedule now.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Schedule id from schedule.list."}}), &["id"]),
            card: Some("Run the schedule {id} now?"),
        },
        Action {
            name: "schedule.work",
            method: "POST",
            path: "/schedules",
            body: Body::Payload("schedule.work"),
            kind: Kind::Write,
            help: "Queue work on a report task from a schedule.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Schedule id from schedule.list."},
                    "task": {"type": "string", "description": "Report task to work on."},
                    "agent": {"type": "string", "description": "Agent that does the work."}
                }),
                &["id"],
            ),
            card: Some("Queue work from the schedule {id} on task \"{task}\" with agent {agent}?"),
        },
        Action {
            name: "decision.get",
            method: "GET",
            path: "/decision",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show who decides: the deep brain or Nixa.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "decision.set",
            method: "POST",
            path: "/decision",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Set who decides: the deep brain or Nixa.",
            inputs: inputs(
                json!({
                    "full": {"type": "string", "enum": ["deep", "nixa"], "description": "Who decides: the deep brain or Nixa."}
                }),
                &["full"],
            ),
            card: Some("Set who decides to {full}?"),
        },
        Action {
            name: "decision.check",
            method: "POST",
            path: "/decision/check",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Test the fast decision trees on a sample and show the report.",
            inputs: inputs(
                json!({
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Flags like --sample TEXT, --kind K, --capability C, --held, --bench."
                    }
                }),
                &[],
            ),
            card: None,
        },
        Action {
            name: "modes.get",
            method: "GET",
            path: "/modes",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the modes view and the number of decisions.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "modes.journal",
            method: "GET",
            path: "/modes/journal",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show past decisions, the last ones or the ones that match words.",
            inputs: inputs(
                json!({
                    "query": {"type": "string", "description": "Words to search. Leave out for the last decisions."},
                    "terminal": {"type": "string", "description": "Only this terminal, when query is left out."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200}
                }),
                &[],
            ),
            card: None,
        },
        Action {
            name: "modes.changes",
            method: "GET",
            path: "/modes/changes",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show what changed since a time, today by default.",
            inputs: inputs(
                json!({
                    "since": {"type": "integer", "description": "Start time in milliseconds. Default start of today."},
                    "project": {"type": "string", "description": "Only this project."}
                }),
                &[],
            ),
            card: None,
        },
        Action {
            name: "modes.scores",
            method: "GET",
            path: "/modes/scores",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the top decision scores and their levels.",
            inputs: inputs(
                json!({
                    "project": {"type": "string", "description": "Only this project."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200}
                }),
                &[],
            ),
            card: None,
        },
        Action {
            name: "modes.records",
            method: "GET",
            path: "/modes/records",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the decision records.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "modes.complaints",
            method: "GET",
            path: "/modes/complaints",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the recent complaints.",
            inputs: inputs(json!({"limit": {"type": "integer", "minimum": 1, "maximum": 200}}), &[]),
            card: None,
        },
    ]
}
