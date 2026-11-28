use serde_json::{Value, json};

fn main() {
    let steps: Vec<Value> = std::env::args().skip(1).filter_map(|a| serde_json::from_str(&a).ok()).collect();
    let steps = if steps.is_empty() { vec![json!({"action": "capabilities"})] } else { steps };
    for step in steps {
        let done = nixa_desktop::execute(&step);
        println!("{}", serde_json::to_string_pretty(&json!({"step": step, "done": done.as_ref().ok(), "error": done.as_ref().err()})).unwrap_or_default());
    }
}
