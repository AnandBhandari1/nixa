use crate::actions::{Action, Body, Kind, inputs};
use serde_json::{Value, json};

fn task_id() -> Value {
    json!({"type": "integer", "minimum": 1, "description": "Task number."})
}

fn flow_name() -> Value {
    json!({"type": "string", "description": "Workflow name from workflow.list."})
}

fn run_task() -> Value {
    json!({"type": "integer", "minimum": 1, "description": "Task number of the workflow run."})
}

fn run_act(name: &'static str, action: &'static str, help: &'static str, card: &'static str) -> Action {
    Action {
        name,
        method: "POST",
        path: "/workflows",
        body: Body::Field("action", action),
        kind: Kind::Write,
        help,
        inputs: inputs(json!({"task": run_task()}), &["task"]),
        card: Some(card),
    }
}

fn flow_act(name: &'static str, action: &'static str, help: &'static str, card: &'static str) -> Action {
    Action {
        name,
        method: "POST",
        path: "/workflows",
        body: Body::Field("action", action),
        kind: Kind::Write,
        help,
        inputs: inputs(json!({"name": flow_name()}), &["name"]),
        card: Some(card),
    }
}

fn tasks() -> Vec<Action> {
    vec![
        Action {
            name: "task.queue",
            method: "GET",
            path: "/tasks/queue",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the task queue and the tasks that wait.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "task.next",
            method: "GET",
            path: "/tasks/next",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the next task to work on in a space.",
            inputs: inputs(json!({"space": {"type": "string", "description": "Space name or id."}}), &["space"]),
            card: None,
        },
        Action {
            name: "task.spaces",
            method: "GET",
            path: "/tasks/spaces",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the spaces with auto start, max doing and lock.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "task.space.set",
            method: "POST",
            path: "/tasks/spaces/{space}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Change the task settings of a space. Give at least one of autoStart, maxDoing or locked.",
            inputs: inputs(
                json!({
                    "space": {"type": "string", "description": "Space name or id."},
                    "autoStart": {"type": "boolean", "description": "Start todo tasks on their own."},
                    "maxDoing": {"type": "integer", "minimum": 1, "description": "How many tasks may run at once."},
                    "locked": {"type": "boolean", "description": "Lock the space so no task starts."}
                }),
                &["space"],
            ),
            card: Some("Change space {space}: auto start {autoStart}, max doing {maxDoing}, locked {locked}?"),
        },
        Action {
            name: "task.resume",
            method: "POST",
            path: "/tasks/resume",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Start the todo tasks again, in one space or in all spaces.",
            inputs: inputs(json!({"space": {"type": "string", "description": "Space id. Leave out for all spaces."}}), &[]),
            card: Some("Start the todo tasks again in space {space} (empty means all spaces)?"),
        },
        Action {
            name: "task.start",
            method: "POST",
            path: "/tasks/{id}/start",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Start a task now.",
            inputs: inputs(json!({"id": task_id()}), &["id"]),
            card: Some("Start task #{id} now?"),
        },
        Action {
            name: "task.continue",
            method: "POST",
            path: "/tasks/{id}/continue",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Continue a task, or hand it over to someone else.",
            inputs: inputs(
                json!({
                    "id": task_id(),
                    "assign": {"type": "string", "description": "Agent, team member (member:ID) or terminal to hand it to. Leave out to keep the same one."}
                }),
                &["id"],
            ),
            card: Some("Continue task #{id}. Hand it to: {assign} (empty means the same one)?"),
        },
        Action {
            name: "task.checkpoint",
            method: "POST",
            path: "/tasks/{id}/checkpoint",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Save the work of a task that a terminal holds.",
            inputs: inputs(
                json!({
                    "id": task_id(),
                    "why": {"type": "string", "description": "Why the checkpoint is made."}
                }),
                &["id"],
            ),
            card: Some("Save a checkpoint of task #{id}: {why}?"),
        },
        Action {
            name: "task.plan",
            method: "GET",
            path: "/tasks/{id}/plan",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the plan of a task.",
            inputs: inputs(json!({"id": task_id()}), &["id"]),
            card: None,
        },
        Action {
            name: "task.plan.set",
            method: "POST",
            path: "/tasks/{id}/plan",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Set the plan steps of a task, mark steps done, or run the plan.",
            inputs: inputs(
                json!({
                    "id": task_id(),
                    "steps": {
                        "type": "array",
                        "description": "New steps in order. Each is a line \"owner | work | output\" or an object.",
                        "items": {
                            "type": ["string", "object"],
                            "properties": {
                                "owner": {"type": "string", "description": "user, nixa or agent:NAME."},
                                "work": {"type": "string", "description": "What the step does."},
                                "output": {"type": "string", "description": "File the step writes."}
                            }
                        }
                    },
                    "done": {"type": "array", "items": {"type": "integer", "minimum": 1}, "description": "Step numbers that are done."},
                    "run": {"type": "boolean", "description": "true runs the plan now."}
                }),
                &["id"],
            ),
            card: Some("Change the plan of task #{id} (steps {steps}, done {done}, run {run})?"),
        },
    ]
}

fn review() -> Vec<Action> {
    vec![
        Action {
            name: "review.view",
            method: "GET",
            path: "/review-flow",
            body: Body::Raw,
            kind: Kind::Read,
            help: "Show the review flow: auto checks, what is being checked and what waits.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "review.auto_checks",
            method: "POST",
            path: "/review-flow/auto-checks",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Turn the automatic project checks on or off.",
            inputs: inputs(json!({"on": {"type": "boolean", "description": "true turns auto checks on."}}), &["on"]),
            card: Some("Turn auto checks on: {on}?"),
        },
        Action {
            name: "review.checks",
            method: "POST",
            path: "/review-flow/checks",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Run a project's own checks now, like typecheck, lint, test or cargo check. At most 150 seconds.",
            inputs: inputs(
                json!({
                    "project": {"type": "string", "description": "Space name or id, or the project folder."},
                    "paths": {"type": "array", "items": {"type": "string"}, "description": "Files or package folders. Leave out for the changed files."},
                    "seconds": {"type": "number", "minimum": 1, "maximum": 150, "description": "Whole time limit in seconds. Default 120, at most 150."}
                }),
                &["project"],
            ),
            card: Some("Run the checks of project {project} now?"),
        },
        Action {
            name: "review.settle",
            method: "POST",
            path: "/review-flow/settle/{id}",
            body: Body::Raw,
            kind: Kind::Write,
            help: "Review a finished task now and move it to done or back to work.",
            inputs: inputs(
                json!({
                    "id": task_id(),
                    "checks": {"type": "boolean", "description": "Run the project checks first. Default true."}
                }),
                &["id"],
            ),
            card: Some("Review task #{id} now and settle it?"),
        },
    ]
}

fn workflows() -> Vec<Action> {
    vec![
        Action {
            name: "workflow.list",
            method: "GET",
            path: "/workflows",
            body: Body::Raw,
            kind: Kind::Read,
            help: "List the saved workflows.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "workflow.saved",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "list"),
            kind: Kind::Read,
            help: "List the saved workflows, the same as workflow.list.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "workflow.read",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "read"),
            kind: Kind::Read,
            help: "Show one saved workflow.",
            inputs: inputs(json!({"name": flow_name()}), &["name"]),
            card: None,
        },
        Action {
            name: "workflow.check",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "check"),
            kind: Kind::Read,
            help: "Check a workflow for problems without saving it.",
            inputs: inputs(
                json!({
                    "workflow": {"type": "object", "description": "The workflow to check."},
                    "name": {"type": "string", "description": "Saved name it would replace, if any."}
                }),
                &["workflow"],
            ),
            card: None,
        },
        Action {
            name: "workflow.runs",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "runs"),
            kind: Kind::Read,
            help: "List the workflow runs.",
            inputs: inputs(json!({}), &[]),
            card: None,
        },
        Action {
            name: "workflow.run",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "run"),
            kind: Kind::Read,
            help: "Show one workflow run by its task number.",
            inputs: inputs(json!({"task": run_task()}), &["task"]),
            card: None,
        },
        Action {
            name: "workflow.put",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "put"),
            kind: Kind::Write,
            help: "Save a workflow, new or changed.",
            inputs: inputs(
                json!({
                    "name": {"type": "string", "description": "Workflow name."},
                    "workflow": {"type": "object", "description": "The whole workflow. Run workflow.check first."}
                }),
                &["name", "workflow"],
            ),
            card: Some("Save the workflow {name}?"),
        },
        flow_act("workflow.delete", "delete", "Delete a saved workflow. Undo puts it back.", "Delete the workflow {name}?"),
        flow_act("workflow.undo", "undo", "Put back the earlier copy of a workflow.", "Put back the earlier copy of the workflow {name}?"),
        run_act("workflow.pause", "pause", "Pause a workflow run.", "Pause the workflow run of task #{task}?"),
        run_act("workflow.resume", "resume", "Resume a paused workflow run.", "Resume the workflow run of task #{task}?"),
        run_act("workflow.cancel", "cancel", "Cancel a workflow run.", "Cancel the workflow run of task #{task}?"),
        run_act("workflow.retry", "retry", "Retry a workflow run that waits.", "Retry the workflow run of task #{task}?"),
        Action {
            name: "workflow.answer",
            method: "POST",
            path: "/workflows",
            body: Body::Field("action", "answer"),
            kind: Kind::Write,
            help: "Answer a workflow run that waits for the user.",
            inputs: inputs(
                json!({
                    "task": run_task(),
                    "text": {"type": "string", "description": "The answer."}
                }),
                &["task", "text"],
            ),
            card: Some("Answer the workflow run of task #{task} with \"{text}\"?"),
        },
    ]
}

pub fn list() -> Vec<Action> {
    let mut out = tasks();
    out.extend(review());
    out.extend(workflows());
    out
}
