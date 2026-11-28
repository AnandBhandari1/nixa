use crate::actions::{Action, Body, Kind, inputs};
use serde_json::{Value, json};

fn space() -> Value {
    json!({"type": "string", "description": "Space name or id. Leave out for the home space."})
}

fn tree_name() -> Value {
    json!({"type": "string", "description": "Tree name from tree.list."})
}

fn box_id() -> Value {
    json!({"type": "string", "description": "Box id, for example r3."})
}

fn drone(word: &str) -> Value {
    json!({"type": "string", "enum": [word], "description": format!("Always \"{word}\".")})
}

fn drone_id() -> Value {
    json!({"type": "string", "description": "Step id from drone.view."})
}

fn drone_step() -> Value {
    json!({"type": "object", "description": "Step JSON. See drone.facts for tests and step kinds."})
}

fn memory() -> Vec<Action> {
    vec![
        Action {
            name: "memory.list",
            method: "GET",
            path: "/memory",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List all memory notes with their ids, and the memory text the brain gets.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "memory.add",
            method: "POST",
            path: "/memory",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Add a memory note.",
            inputs: inputs(json!({"text": {"type": "string", "description": "The note to remember."}}), &["text"]),
            card: Some("Remember this note: \"{text}\"?"),
        },
        Action {
            name: "memory.remove",
            method: "DELETE",
            path: "/memory/{id}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Delete a memory note by its id.",
            inputs: inputs(json!({"id": {"type": "integer", "description": "Note id from memory.list."}}), &["id"]),
            card: Some("Delete memory note {id}?"),
        },
    ]
}

fn deep() -> Vec<Action> {
    vec![
        Action {
            name: "deep.view",
            method: "GET",
            path: "/deep",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the deep brain seats and their open requests.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "deep.start",
            method: "POST",
            path: "/deep/start",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Start a deep brain agent for a space.",
            inputs: inputs(
                json!({
                    "brain": {"type": "string", "description": "Brain kind. Default claude."},
                    "space": space()
                }),
                &[],
            ),
            card: Some("Start a deep brain ({brain}, default claude) for space {space} (default home space)?"),
        },
        Action {
            name: "deep.ask",
            method: "POST",
            path: "/deep/ask",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Send a hard decision to the deep brain of a space.",
            inputs: inputs(
                json!({
                    "question": {"type": "string", "description": "The question to decide."},
                    "options": {"type": "array", "items": {"type": "string"}, "description": "The choices."},
                    "notes": {"type": "string", "description": "Facts the deep brain needs."},
                    "kind": {"type": "string", "description": "Request kind, for example fix, check, assign, blocker, close, model or work."},
                    "wait": {"type": "integer", "minimum": 0, "description": "Milliseconds to wait for the answer. Leave out for the default."},
                    "brain": {"type": "string", "description": "Brain id to ask."},
                    "space": space()
                }),
                &["question"],
            ),
            card: Some("Ask the deep brain of space {space}: \"{question}\"?"),
        },
        Action {
            name: "deep.digest",
            method: "POST",
            path: "/deep/digest",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Get the deep brain digest of a space.",
            inputs: inputs(json!({"space": space()}), &[]),
            card: None,
        },
    ]
}

fn trees() -> Vec<Action> {
    vec![
        Action {
            name: "tree.list",
            method: "GET",
            path: "/trees",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the rule trees.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "tree.read",
            method: "GET",
            path: "/trees/{name}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Read one whole rule tree.",
            inputs: inputs(json!({"name": tree_name()}), &["name"]),
            card: None,
        },
        Action {
            name: "tree.put",
            method: "PUT",
            path: "/trees/{name}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Save a whole rule tree.",
            inputs: inputs(
                json!({
                    "name": tree_name(),
                    "tree": {"type": "object", "description": "The whole tree JSON: name, title, when, start, boxes."}
                }),
                &["name", "tree"],
            ),
            card: Some("Replace the rule tree {name} with the given tree?"),
        },
        Action {
            name: "tree.delete",
            method: "DELETE",
            path: "/trees/{name}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Delete a rule tree. A copy stays in the history folder.",
            inputs: inputs(json!({"name": tree_name()}), &["name"]),
            card: Some("Delete the rule tree {name}?"),
        },
        Action {
            name: "tree.undo",
            method: "POST",
            path: "/trees/{name}/undo",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Put a rule tree back to the version before its last edit.",
            inputs: inputs(json!({"name": tree_name()}), &["name"]),
            card: Some("Undo the last edit of the rule tree {name}?"),
        },
        Action {
            name: "watch.dry",
            method: "POST",
            path: "/watch/dry",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Dry run the watch tree, or a given tree, over the current tasks and terminals.",
            inputs: inputs(json!({"tree": {"type": "object", "description": "Tree JSON to test. Leave out for the saved watch tree."}}), &[]),
            card: None,
        },
        Action {
            name: "tree.box",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "box"),
            kind: Kind::Read,
            help: "Show one box of a rule tree and its next lines.",
            inputs: inputs(json!({"name": tree_name(), "id": {"type": "string", "description": "Box id. Leave out for the start box."}}), &["name"]),
            card: None,
        },
        Action {
            name: "tree.walk",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "walk"),
            kind: Kind::Read,
            help: "Dry run the watch tree, or a given tree, and give the walk as text.",
            inputs: inputs(json!({"tree": {"type": "object", "description": "Tree JSON to test. Leave out for the saved watch tree."}}), &[]),
            card: None,
        },
        Action {
            name: "tree.create",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "create"),
            kind: Kind::Write,
            help: "Create a new rule tree.",
            inputs: inputs(
                json!({
                    "tree": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string", "description": "Short tree name."},
                            "title": {"type": "string"},
                            "when": {"type": "string", "description": "When the tree is used."},
                            "start": {"type": "string", "description": "Start box id."},
                            "boxes": {"type": "object", "description": "Boxes by id, each with text and next."}
                        },
                        "required": ["name", "boxes"]
                    }
                }),
                &["tree"],
            ),
            card: Some("Create the rule tree {tree.name}: {tree.when}?"),
        },
        Action {
            name: "tree.add",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "add"),
            kind: Kind::Write,
            help: "Add a box to a rule tree, under another box when from is given.",
            inputs: inputs(
                json!({
                    "name": tree_name(),
                    "text": {"type": "string", "description": "One plain English rule."},
                    "from": {"type": "string", "description": "Box id the new box goes under."},
                    "word": {"type": "string", "description": "Answer word on the line from that box, for example Yes."}
                }),
                &["name", "text"],
            ),
            card: Some("Add the box \"{text}\" to the rule tree {name}?"),
        },
        Action {
            name: "tree.change",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "change"),
            kind: Kind::Write,
            help: "Change a box of a rule tree. Give box, or text and/or next.",
            inputs: inputs(
                json!({
                    "name": tree_name(),
                    "id": box_id(),
                    "box": {"type": "object", "description": "The whole new box."},
                    "text": {"type": "string", "description": "New box text."},
                    "next": {"type": "object", "description": "New next lines, word to box id."}
                }),
                &["name", "id"],
            ),
            card: Some("Change box {id} of the rule tree {name}?"),
        },
        Action {
            name: "tree.remove",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "remove"),
            kind: Kind::Write,
            help: "Remove a box from a rule tree, with the lines that point to it.",
            inputs: inputs(json!({"name": tree_name(), "id": box_id()}), &["name", "id"]),
            card: Some("Remove box {id} from the rule tree {name}?"),
        },
        Action {
            name: "tree.meta",
            method: "POST",
            path: "/agent/trees",
            body: Body::Field("action", "meta"),
            kind: Kind::Write,
            help: "Change the title, when text or start box of a rule tree.",
            inputs: inputs(
                json!({
                    "name": tree_name(),
                    "title": {"type": "string"},
                    "when": {"type": "string", "description": "When the tree is used."},
                    "start": {"type": "string", "description": "Start box id."}
                }),
                &["name"],
            ),
            card: Some("Change the rule tree {name}: title {title}, when {when}, start {start}?"),
        },
    ]
}

fn knowledge() -> Vec<Action> {
    vec![
        Action {
            name: "knowledge.stats",
            method: "GET",
            path: "/knowledge",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the knowledge index: chunks, health, folders and sources.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "knowledge.index",
            method: "POST",
            path: "/knowledge/index",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Index the knowledge folders now.",
            inputs: inputs(json!({}), &[]),
            card: Some("Index the knowledge folders now?"),
        },
        Action {
            name: "knowledge.forget_chat",
            method: "POST",
            path: "/knowledge/forget_chat",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Remove the chat history from the knowledge index.",
            inputs: inputs(json!({}), &[]),
            card: Some("Remove the chat history from the knowledge index?"),
        },
        Action {
            name: "knowledge.folders",
            method: "GET",
            path: "/knowledge/folders",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the folders the knowledge index reads.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "knowledge.folders_save",
            method: "POST",
            path: "/knowledge/folders",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Set the folders the knowledge index reads.",
            inputs: inputs(json!({"folders": {"type": "array", "items": {"type": "string"}, "description": "Full folder paths."}}), &["folders"]),
            card: Some("Set the knowledge folders to {folders}?"),
        },
        Action {
            name: "knowledge.skills",
            method: "GET",
            path: "/knowledge/skills",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the installed skills and the ones Nixa may use.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "knowledge.skills_save",
            method: "POST",
            path: "/knowledge/skills",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Set the skills Nixa may use.",
            inputs: inputs(json!({"allowed": {"type": "array", "items": {"type": "string"}, "description": "Skill names."}}), &["allowed"]),
            card: Some("Set the skills Nixa may use to {allowed}?"),
        },
    ]
}

fn drones() -> Vec<Action> {
    vec![
        Action {
            name: "drone.view",
            method: "GET",
            path: "/drones",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the drone tree, its steps and timing.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "drone.snapshot",
            method: "GET",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Get the full drone store snapshot. Give action=snapshot.",
            inputs: inputs(json!({"action": drone("snapshot")}), &["action"]),
            card: None,
        },
        Action {
            name: "drone.facts",
            method: "GET",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the facts, tests and step kinds drones can use. Give action=facts.",
            inputs: inputs(json!({"action": drone("facts")}), &["action"]),
            card: None,
        },
        Action {
            name: "drone.history",
            method: "GET",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the last drone edits. Give action=history.",
            inputs: inputs(json!({"action": drone("history")}), &["action"]),
            card: None,
        },
        Action {
            name: "drone.read",
            method: "GET",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the drone tree as pretty text. Give action=read.",
            inputs: inputs(json!({"action": drone("read")}), &["action"]),
            card: None,
        },
        Action {
            name: "drone.dry_run",
            method: "GET",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Dry run the drones over the current tasks and terminals. Give action=dry_run.",
            inputs: inputs(json!({"action": drone("dry_run")}), &["action"]),
            card: None,
        },
        Action {
            name: "drone.dry_run_step",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Dry run the drones with one step changed. Give action=dry_run.",
            inputs: inputs(json!({"action": drone("dry_run"), "id": drone_id(), "step": drone_step()}), &["action", "id", "step"]),
            card: Some("Drone {action} with step {id} set to {step}?"),
        },
        Action {
            name: "drone.run",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Run the drones once now. Give action=run.",
            inputs: inputs(json!({"action": drone("run")}), &["action"]),
            card: Some("Drone {action}: run the drones once now? They may report findings to the brain."),
        },
        Action {
            name: "drone.add",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Add a drone step. Give action=add.",
            inputs: inputs(json!({"action": drone("add"), "id": drone_id(), "step": drone_step()}), &["action", "id", "step"]),
            card: Some("Drone {action}: step {id} with {step}?"),
        },
        Action {
            name: "drone.change",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Change a drone step. Give action=change.",
            inputs: inputs(json!({"action": drone("change"), "id": drone_id(), "step": drone_step()}), &["action", "id", "step"]),
            card: Some("Drone {action}: step {id} to {step}?"),
        },
        Action {
            name: "drone.remove",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Remove a drone step. Give action=remove.",
            inputs: inputs(json!({"action": drone("remove"), "id": drone_id()}), &["action", "id"]),
            card: Some("Drone {action}: step {id}?"),
        },
        Action {
            name: "drone.on",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Turn the drones on, or one step when id is given. Give action=on.",
            inputs: inputs(json!({"action": drone("on"), "id": drone_id()}), &["action"]),
            card: Some("Drone {action}: step {id} (blank means all drones)?"),
        },
        Action {
            name: "drone.off",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Turn the drones off, or one step when id is given. Give action=off.",
            inputs: inputs(json!({"action": drone("off"), "id": drone_id()}), &["action"]),
            card: Some("Drone {action}: step {id} (blank means all drones)?"),
        },
        Action {
            name: "drone.timing",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Set how often drones check and when they remind. Give action=timing.",
            inputs: inputs(
                json!({
                    "action": drone("timing"),
                    "every": {"type": "integer", "minimum": 1, "maximum": 60, "description": "Minutes between drone checks."},
                    "remind": {"type": "integer", "minimum": 1, "maximum": 240, "description": "Minutes before one reminder."}
                }),
                &["action"],
            ),
            card: Some("Drone {action}: check every {every} minutes, remind after {remind} minutes (a blank one stays as it is)?"),
        },
        Action {
            name: "drone.undo",
            method: "POST",
            path: "/drones/{action}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Undo the last drone edit. Give action=undo.",
            inputs: inputs(json!({"action": drone("undo")}), &["action"]),
            card: Some("Drone {action}: undo the last drone edit?"),
        },
    ]
}

pub fn list() -> Vec<Action> {
    [memory(), deep(), trees(), knowledge(), drones()].concat()
}
