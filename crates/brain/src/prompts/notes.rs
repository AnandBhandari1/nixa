pub fn auto_note() -> String {
    " Decide it yourself, do not wait for the user. Read the agent's screen and its work first with nixa_term_read, weigh every option against the job and its trade-offs (never blindly pick option 1 or the one marked Recommended), and ask nixa_deep_ask first if you are truly unsure. Then call nixa_answer_prompt with decide true and a short reason why the chosen option fits. Allow once; a choice always is sent as yes. Then say in one or two short sentences what you decided.".to_string()
}

pub fn deep_note() -> &'static str {
    " This question comes from your deep brain terminal while Decide it yourself: answer it with its own recommended option (nixa_answer_prompt with decide true and a short reason), then say in one short sentence what you picked. Do not pause for approval. Record the decision and preserve the user's Stop, off and manual controls."
}

pub fn deep_routed_note(ask: &str) -> String {
    format!(" The deep brain decides this one as request {ask}. Do not answer it, do not ask the user and do not call nixa_deep_ask: Nixa sends the deep brain's pick to the agent once. Say in one short line that the deep brain decides it.")
}

pub fn deep_late_note(ask: &str, prompt: &str) -> String {
    format!("The deep brain answered {ask} after prompt {prompt} was closed or changed. Nothing was sent.")
}

pub fn same_note(id: &str, said: &str) -> String {
    format!(" The same agent asked the same thing before ({id}) and got the answer {}. Check its screen before you answer again, so it does not go round in a loop.", serde_json::to_string(said).unwrap_or_default())
}

pub fn rule_note(id: u64, answer: &str) -> String {
    format!(" The user's live answer rule {id} fits this question. Its answer is {}. Use it with nixa_answer_prompt when the screen still asks this, and say that the rule answered.", serde_json::to_string(answer).unwrap_or_default())
}

pub fn on_screen_note(id: &str, terminal: &str) -> String {
    format!("Note: The user answered prompt {id} in terminal {terminal} on its screen. It is closed. Do not ask about it or answer it again.")
}

pub fn clicked_note(id: &str, said: &str) -> String {
    format!("Note: The user answered prompt {id} with a click on its card: {said}. It is closed. Do not ask about it or answer it again.")
}
