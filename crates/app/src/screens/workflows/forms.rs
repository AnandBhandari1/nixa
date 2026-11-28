use super::flow::{self, Workflow};
use super::view::{BAD_NAME, Modal, WorkflowsView, set, value};
use gpui::{Context, Focusable};
use serde_json::json;

impl WorkflowsView {
    pub fn saving_form(&self) -> bool {
        matches!(self.busy.as_str(), "create" | "copy")
    }

    fn form_problem(&mut self, text: &str, cx: &mut Context<Self>) {
        self.form_error = text.into();
        cx.notify();
    }

    fn valid_new_name(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        if !flow::valid_name(name) {
            self.form_problem(BAD_NAME, cx);
            return false;
        }
        if self.names.iter().any(|n| n.name == name) {
            self.form_problem("A workflow with that name exists.", cx);
            return false;
        }
        true
    }

    pub fn create_flow(&mut self, cx: &mut Context<Self>) {
        if !self.busy.is_empty() {
            return;
        }
        let name = value(&self.inputs.name, cx).trim().to_string();
        if !self.valid_new_name(&name, cx) {
            return;
        }
        let when = value(&self.inputs.when, cx);
        if when.trim().is_empty() {
            self.form_problem("Write when to use the workflow.", cx);
            return;
        }
        let title = value(&self.inputs.title, cx);
        let title = if title.trim().is_empty() {
            &name
        } else {
            title.trim()
        };
        self.save_new("create", flow::template(&name, title, when.trim()), cx);
    }

    pub fn copy_flow(&mut self, cx: &mut Context<Self>) {
        if !self.busy.is_empty() {
            return;
        }
        let name = value(&self.inputs.copy, cx).trim().to_string();
        if !self.valid_new_name(&name, cx) {
            return;
        }
        if let Some(flow) = &self.flow {
            self.save_new("copy", flow::copy_of(flow, &name, &self.name), cx);
        }
    }

    fn save_new(&mut self, action: &str, flow: Workflow, cx: &mut Context<Self>) {
        let name = flow.name.clone();
        let args = json!({"name": name, "workflow": flow::to_value(&flow)});
        self.busy = action.into();
        self.form_error.clear();
        cx.notify();
        self.job(
            cx,
            move |call| call("put", args),
            move |view, out, cx| {
                view.busy.clear();
                match out {
                    Ok(_) => {
                        view.close_modal(cx);
                        view.load_names(cx);
                        view.open_flow(&name, false, cx);
                    }
                    Err(error) => view.form_error = error,
                }
            },
        );
    }

    pub fn workflow_details(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(flow) = &self.flow else {
            return;
        };
        set(&self.inputs.title, &flow.title, cx);
        set(&self.inputs.when, &flow.when, cx);
        set(&self.inputs.step_limit, &flow.max_steps().to_string(), cx);
        self.form_error.clear();
        self.modal = Some(Modal::Details);
        self.focus_next = Some(self.inputs.title.focus_handle(cx));
        cx.notify();
    }

    pub fn box_details(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(node) = self.flow.as_ref().and_then(|flow| flow.nodes.get(&id)) else {
            return;
        };
        set(&self.inputs.box_title, &node.title, cx);
        self.form_error.clear();
        self.modal = Some(Modal::Box(id));
        self.focus_next = Some(self.inputs.box_title.focus_handle(cx));
        cx.notify();
    }

    pub fn toggle_box(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.edit(cx, move |flow, _| {
            if let Some(node) = flow.nodes.get_mut(&id).filter(|node| node.kind == "stage") {
                node.off = !node.off;
            }
        });
    }

    pub fn apply_details(&mut self, cx: &mut Context<Self>) {
        match self.modal.clone() {
            Some(Modal::Details) => {
                let title = value(&self.inputs.title, cx).trim().to_string();
                let when = value(&self.inputs.when, cx).trim().to_string();
                let steps = value(&self.inputs.step_limit, cx);
                let Ok(steps @ 1..=500) = steps.trim().parse::<u32>() else {
                    return self
                        .form_problem("Maximum steps must be a whole number from 1 to 500.", cx);
                };
                if title.is_empty() || when.is_empty() {
                    return self.form_problem("Enter a title and when to use the workflow.", cx);
                }
                self.edit(cx, move |flow, _| {
                    flow.title = title;
                    flow.when = when;
                    flow.set_max_steps(steps);
                });
            }
            Some(Modal::Box(id)) => {
                let title = value(&self.inputs.box_title, cx).trim().to_string();
                if title.is_empty() {
                    return self.form_problem("Enter a box title.", cx);
                }
                self.edit(cx, move |flow, _| {
                    if let Some(node) = flow.nodes.get_mut(&id) {
                        node.title = title;
                    }
                });
            }
            _ => return,
        }
        self.close_modal(cx);
    }
}
