use crate::actions::{Action, Body, Kind, inputs};
use serde_json::json;

pub fn list() -> Vec<Action> {
    let mut out = desktop();
    out.extend(browser());
    out.extend(connectors());
    out.extend(extras());
    out.extend(assistant());
    out
}

fn desktop() -> Vec<Action> {
    vec![
        Action {
            name: "desktop.view",
            method: "GET",
            path: "/desktop",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the open hands card, the recent desktop steps and if screen reads are allowed.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "desktop.answer",
            method: "POST",
            path: "/desktop/answer",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Answer yes or no on the open hands card.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Card id from desktop.view."},
                    "answer": {"type": "string", "enum": ["yes", "no"], "description": "yes runs the step, no stops it."}
                }),
                &["id", "answer"],
            ),
            card: Some("Answer {answer} on the hands card {id}?"),
        },
        Action {
            name: "desktop.reads",
            method: "POST",
            path: "/desktop/reads",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Allow or stop screen reads without a card each time.",
            inputs: inputs(json!({"allowReads": {"type": "boolean", "description": "true allows screen reads, false asks each time."}}), &["allowReads"]),
            card: Some("Set allow screen reads to {allowReads}?"),
        },
    ]
}

fn browser() -> Vec<Action> {
    vec![
        Action {
            name: "browser.view",
            method: "GET",
            path: "/browser",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the browser state: on or off, the browser command and the download choice.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "browser.command",
            method: "POST",
            path: "/browser",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Save the command that starts the browser helper.",
            inputs: inputs(json!({"command": {"type": "string", "description": "The browser helper command."}}), &["command"]),
            card: Some("Save the browser command \"{command}\"?"),
        },
        Action {
            name: "browser.on",
            method: "POST",
            path: "/browser",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Turn browser actions on or off. Off also stops the browser.",
            inputs: inputs(json!({"on": {"type": "boolean", "description": "true turns browser actions on, false turns them off."}}), &["on"]),
            card: Some("Turn browser actions on: {on}?"),
        },
        Action {
            name: "browser.download",
            method: "POST",
            path: "/browser",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Allow or refuse the package download that the browser command needs.",
            inputs: inputs(
                json!({"download": {"type": "boolean", "description": "true allows the download, false refuses it."}}),
                &["download"],
            ),
            card: Some("Set the browser package download to {download}?"),
        },
        Action {
            name: "browser.stop",
            method: "POST",
            path: "/browser",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Stop the running browser now.",
            inputs: inputs(json!({"stop": {"type": "boolean", "const": true, "description": "Must be true."}}), &["stop"]),
            card: Some("Stop the browser now?"),
        },
    ]
}

fn connectors() -> Vec<Action> {
    vec![
        Action {
            name: "connector.list",
            method: "GET",
            path: "/connectors",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the installed connectors, their status, actions and the open connector card.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "connector.calls",
            method: "GET",
            path: "/connectors/calls",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the latest connector calls, newest first.",
            inputs: inputs(
                json!({
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "How many calls. Default 20."},
                    "connector": {"type": "string", "description": "Only calls of this connector."}
                }),
                &[],
            ),
            card: None,
        },
        Action {
            name: "connector.trust",
            method: "POST",
            path: "/connectors",
            body: Body::Field("type", "trust"),
            kind: Kind::Write,
            help: "Mark a connector as trusted. Every step already runs without a card.",
            inputs: inputs(json!({"connector": {"type": "string", "description": "Connector name from connector.list."}}), &["connector"]),
            card: Some("Trust the connector {connector}?"),
        },
        Action {
            name: "connector.untrust",
            method: "POST",
            path: "/connectors",
            body: Body::Field("type", "untrust"),
            kind: Kind::Write,
            help: "Stop trusting a connector, so its outside writes ask again.",
            inputs: inputs(json!({"connector": {"type": "string", "description": "Connector name from connector.list."}}), &["connector"]),
            card: Some("Stop trusting the connector {connector}?"),
        },
        Action {
            name: "connector.secret_remove",
            method: "POST",
            path: "/connectors",
            body: Body::Field("type", "secret_remove"),
            kind: Kind::Write,
            help: "Remove a connector secret from the Keychain.",
            inputs: inputs(json!({"entry": {"type": "string", "description": "Secret entry name from connector.list."}}), &["entry"]),
            card: Some("Remove the connector secret {entry}?"),
        },
        Action {
            name: "connector.answer",
            method: "POST",
            path: "/connectors",
            body: Body::Field("type", "answer"),
            kind: Kind::Write,
            help: "Answer yes or no on the open connector card.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Card id from connector.list."},
                    "yes": {"type": "boolean", "description": "true runs the call, false stops it."}
                }),
                &["id", "yes"],
            ),
            card: Some("Answer yes: {yes} on the connector card {id}?"),
        },
    ]
}

fn extras() -> Vec<Action> {
    let kind = json!({"type": "string", "enum": ["gap", "refusal"], "description": "gap or refusal. Leave out for both."});
    vec![
        Action {
            name: "extras.view",
            method: "GET",
            path: "/extras",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show usage, goals, the debate state and the opinion count.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "extras.usage",
            method: "GET",
            path: "/extras/usage",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the brain usage numbers.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "extras.goals",
            method: "GET",
            path: "/extras/goals",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the goal lines for the last days.",
            inputs: inputs(json!({"days": {"type": "integer", "minimum": 1, "maximum": 30, "description": "How many days back, from 1 to 30. Leave out for the default."}}), &[]),
            card: None,
        },
        Action {
            name: "gap.list",
            method: "GET",
            path: "/extras/gaps",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the tool gaps and refusals the brain noted.",
            inputs: inputs(json!({"kind": kind}), &[]),
            card: None,
        },
        Action {
            name: "gap.remove",
            method: "POST",
            path: "/extras/gaps",
            body: Body::Field("action", "remove"),
            kind: Kind::Write,
            help: "Remove one tool gap.",
            inputs: inputs(json!({"id": {"type": "integer", "description": "Gap id from gap.list."}}), &["id"]),
            card: Some("Remove the tool gap #{id}?"),
        },
        Action {
            name: "gap.clear",
            method: "POST",
            path: "/extras/gaps",
            body: Body::Field("action", "clear"),
            kind: Kind::Write,
            help: "Clear all tool gaps, or only one kind.",
            inputs: inputs(json!({"kind": kind}), &[]),
            card: Some("Clear the tool gaps of kind {kind} (empty means all kinds)?"),
        },
        Action {
            name: "gap.file",
            method: "POST",
            path: "/extras/gaps",
            body: Body::Field("action", "file"),
            kind: Kind::Write,
            help: "File a tool gap as a task.",
            inputs: inputs(
                json!({
                    "id": {"type": "integer", "description": "Gap id from gap.list."},
                    "confirm": {"type": "boolean", "description": "true files it even when private words or secrets were found. Default false."}
                }),
                &["id"],
            ),
            card: Some("File the tool gap #{id} as a task?"),
        },
        Action {
            name: "extras.peer",
            method: "GET",
            path: "/extras/peer/{terminal}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the programs running in a terminal.",
            inputs: inputs(json!({"terminal": {"type": "string", "description": "Terminal id."}}), &["terminal"]),
            card: None,
        },
        Action {
            name: "model_pick.show",
            method: "GET",
            path: "/extras/model-pick/{task}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the saved model pick for a task.",
            inputs: inputs(json!({"task": {"type": "integer", "minimum": 1, "description": "Task number."}}), &["task"]),
            card: None,
        },
        Action {
            name: "model_pick.ask",
            method: "POST",
            path: "/extras/model-pick/{task}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Ask the brain to pick a model for a task.",
            inputs: inputs(json!({"task": {"type": "integer", "minimum": 1, "description": "Task number."}}), &["task"]),
            card: Some("Ask the brain to pick a model for task #{task}?"),
        },
    ]
}

fn assistant() -> Vec<Action> {
    let string = json!({"type": "string"});
    vec![
        Action {
            name: "assistant.view",
            method: "GET",
            path: "/assistant",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the morning briefing, the open reminders and the ready sources.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "assistant.ask",
            method: "POST",
            path: "/assistant",
            body: Body::Field("action", "ask"),
            kind: Kind::Write,
            help: "Ask the assistant a quick question. The answer shows in Nixa chat.",
            inputs: inputs(json!({"question": {"type": "string", "description": "The question."}}), &["question"]),
            card: Some("Ask the assistant \"{question}\"?"),
        },
        Action {
            name: "assistant.briefing",
            method: "POST",
            path: "/assistant",
            body: Body::Field("action", "briefing"),
            kind: Kind::Write,
            help: "Save or change the daily morning briefing.",
            inputs: inputs(
                json!({
                    "time": {"type": "string", "description": "24-hour time, default 08:00."},
                    "place": {"type": "string", "description": "City for the weather."},
                    "topics": {"type": "array", "items": string, "description": "News topics."},
                    "enabled": {"type": "boolean", "description": "Default true. false pauses it."}
                }),
                &[],
            ),
            card: Some("Save the morning briefing: time {time}, place {place}, topics {topics}, enabled {enabled} (empty time, place or topics keep the saved value, empty enabled means on)?"),
        },
        Action {
            name: "assistant.briefing_on",
            method: "POST",
            path: "/assistant",
            body: Body::Field("action", "briefing_on"),
            kind: Kind::Write,
            help: "Turn the morning briefing on or off.",
            inputs: inputs(json!({"enabled": {"type": "boolean", "description": "Default true. false pauses it."}}), &[]),
            card: Some("Turn the morning briefing on: {enabled}?"),
        },
        Action {
            name: "assistant.remind",
            method: "POST",
            path: "/assistant",
            body: Body::Field("action", "remind"),
            kind: Kind::Write,
            help: "Set a personal reminder.",
            inputs: inputs(
                json!({
                    "text": {"type": "string", "description": "What to remind about."},
                    "when": {"type": "string", "description": "For example: at 5, tomorrow at 9 am, in 20 minutes."},
                    "task": {"type": "integer", "minimum": 1, "description": "Optional task number to follow up on."}
                }),
                &["text", "when"],
            ),
            card: Some("Set the reminder \"{text}\" {when}?"),
        },
        Action {
            name: "assistant.remind_cancel",
            method: "POST",
            path: "/assistant",
            body: Body::Field("action", "remind_cancel"),
            kind: Kind::Write,
            help: "Cancel a reminder.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Reminder id from assistant.view."}}), &["id"]),
            card: Some("Cancel the reminder {id}?"),
        },
    ]
}
