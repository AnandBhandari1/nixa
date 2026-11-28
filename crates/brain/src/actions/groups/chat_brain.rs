use crate::actions::{Action, Body, Kind, inputs};
use serde_json::json;

pub fn list() -> Vec<Action> {
    let mut out = chat();
    out.extend(brains());
    out.extend(prompts());
    out.extend(rules());
    out.extend(content());
    out
}

fn chat() -> Vec<Action> {
    vec![
        Action {
            name: "chat.list",
            method: "GET",
            path: "/chat",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the Nixa chat messages and say if the brain is busy.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "chat.say",
            method: "POST",
            path: "/chat",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Send a message to Nixa chat.",
            inputs: inputs(
                json!({
                    "text": {"type": "string", "description": "The message."},
                    "typed": {"type": "boolean", "description": "true for typed, false for heard speech. Default true."}
                }),
                &["text"],
            ),
            card: Some("Send \"{text}\" to Nixa chat?"),
        },
        Action {
            name: "chat.stop",
            method: "POST",
            path: "/chat/interrupt",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Stop the brain turn that runs now.",
            inputs: inputs(json!({}), &[]),
            card: Some("Stop the brain turn that runs now?"),
        },
        Action {
            name: "chat.clean",
            method: "POST",
            path: "/chat/clean",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Clear the chat and start the brain fresh with a short recap.",
            inputs: inputs(json!({}), &[]),
            card: Some("Clear the Nixa chat and start the brain fresh?"),
        },
        Action {
            name: "chat.context",
            method: "GET",
            path: "/context",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show how full the brain context is and what is folded.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "chat.compact",
            method: "POST",
            path: "/compact",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Compact the brain context now.",
            inputs: inputs(json!({}), &[]),
            card: Some("Compact the brain context now?"),
        },
    ]
}

fn brains() -> Vec<Action> {
    vec![
        Action {
            name: "brain.list",
            method: "GET",
            path: "/brains",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the saved brains, the CLI brains and the brain picked now.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "brain.save",
            method: "POST",
            path: "/brains",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Add a saved brain, or change one when id is given. The server is checked first.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Saved brain id to change. Leave out to add a new one."},
                    "name": {"type": "string", "description": "The brain name."},
                    "kind": {"type": "string", "enum": ["openai", "anthropic"], "description": "The API kind."},
                    "baseUrl": {"type": "string", "description": "Server URL. Default is the kind's own URL."},
                    "keyEnv": {"type": "string", "description": "Name of an env variable that holds the key."},
                    "model": {"type": "string", "description": "Model id."}
                }),
                &["name", "kind"],
            ),
            card: Some("Save the brain {name} ({kind}, id {id}) with server {baseUrl} and model {model}?"),
        },
        Action {
            name: "brain.delete",
            method: "DELETE",
            path: "/brains/{id}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Delete a saved brain and its keychain key.",
            inputs: inputs(json!({"id": {"type": "string", "description": "Saved brain id from brain.list."}}), &["id"]),
            card: Some("Delete the saved brain {id} and its key?"),
        },
        Action {
            name: "brain.use",
            method: "POST",
            path: "/brain/choice",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Pick the brain Nixa uses.",
            inputs: inputs(
                json!({"choice": {"type": "string", "description": "A choice from brain.list, like api:ID or a CLI brain word."}}),
                &["choice"],
            ),
            card: Some("Switch the Nixa brain to {choice}?"),
        },
        Action {
            name: "brain.models",
            method: "GET",
            path: "/brain/models",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the models the picked brain offers.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "brain.model",
            method: "POST",
            path: "/brain/model",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Set the model or the effort of the picked brain.",
            inputs: inputs(
                json!({
                    "model": {"type": "string", "description": "Model id from brain.models."},
                    "effort": {"type": "string", "description": "Effort word. Empty text clears it."}
                }),
                &[],
            ),
            card: Some("Set the brain model to {model} with effort {effort}?"),
        },
    ]
}

fn prompts() -> Vec<Action> {
    vec![
        Action {
            name: "prompt.list",
            method: "GET",
            path: "/prompts",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the open agent questions, permissions and plans.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "prompt.answer",
            method: "POST",
            path: "/prompts/{id}/answer",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Answer an open agent question, permission or plan.",
            inputs: inputs(
                json!({
                    "id": {"type": "string", "description": "Prompt id from prompt.list."},
                    "choice": {"type": "string", "enum": ["yes", "always", "no"], "description": "For a permission or plan."},
                    "options": {"type": "array", "items": {"type": "integer", "minimum": 1}, "description": "Option numbers from 1."},
                    "text": {"type": "string", "description": "Typed answer."},
                    "indexes": {"type": "array", "items": {"type": "integer", "minimum": 0}, "description": "Click these option indexes instead."},
                    "decide": {"type": "boolean", "description": "true when Nixa decides without the user."},
                    "reason": {"type": "string", "description": "Why, when decide is true."}
                }),
                &["id"],
            ),
            card: Some("Answer agent prompt {id} with choice {choice}, options {options}, indexes {indexes}, text {text}, decide {decide}?"),
        },
    ]
}

fn rules() -> Vec<Action> {
    vec![
        Action {
            name: "rule.list",
            method: "GET",
            path: "/answer-rules",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the answer rules.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "rule.save",
            method: "POST",
            path: "/answer-rules",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Add an answer rule, or change one when id is given.",
            inputs: inputs(
                json!({
                    "id": {"type": "integer", "description": "Rule id to change. Leave out to add a new one."},
                    "project": {"type": "string", "description": "Project folder, or the all word for every project."},
                    "ask": {"type": "string", "description": "The question the rule matches."},
                    "answer": {"type": "string", "description": "The answer to give."},
                    "status": {"type": "string", "enum": ["trial", "offer", "live", "off"], "description": "Default trial."},
                    "expected": {"type": "string", "description": "Stamp from rule.list, to catch a change made meanwhile."}
                }),
                &["project", "ask", "answer"],
            ),
            card: Some("Save the answer rule for {project}: \"{ask}\" answered with \"{answer}\" ({status})?"),
        },
        Action {
            name: "rule.check",
            method: "POST",
            path: "/answer-rules/check",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Check which answer rule matches a question in a folder.",
            inputs: inputs(
                json!({
                    "folder": {"type": "string", "description": "Project folder."},
                    "question": {"type": "string", "description": "The question text."}
                }),
                &["folder", "question"],
            ),
            card: None,
        },
        Action {
            name: "rule.remove",
            method: "POST",
            path: "/answer-rules/{id}/remove",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Remove an answer rule.",
            inputs: inputs(
                json!({
                    "id": {"type": "integer", "description": "Rule id from rule.list."},
                    "expected": {"type": "string", "description": "Stamp from rule.list, to catch a change made meanwhile."}
                }),
                &["id"],
            ),
            card: Some("Remove the answer rule {id}?"),
        },
    ]
}

fn content() -> Vec<Action> {
    vec![
        Action {
            name: "content.get",
            method: "GET",
            path: "/content/{document}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Read the whole memory or rules document with its hash.",
            inputs: inputs(json!({"document": {"type": "string", "enum": ["memory", "rules"], "description": "memory or rules."}}), &["document"]),
            card: None,
        },
        Action {
            name: "content.save",
            method: "POST",
            path: "/content/{document}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Replace the whole memory or rules document.",
            inputs: inputs(
                json!({
                    "document": {"type": "string", "enum": ["memory", "rules"], "description": "memory or rules."},
                    "text": {"type": "string", "description": "The complete new text."},
                    "expectedHash": {"type": "string", "description": "Hash from content.get. It is needed, to catch a change made meanwhile."}
                }),
                &["document", "text", "expectedHash"],
            ),
            card: Some("Replace the whole {document} document?"),
        },
    ]
}
