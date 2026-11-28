pub const RUBRIC: &str = "declutter";
pub const RUBRICS: [&str; 1] = [RUBRIC];
pub const QUESTION: &str = "Review this screen with the declutter check.";
pub const COUNT_WORD: &str = "DECLUTTER:";

pub const STEPS: [(&str, &str); 4] = [
    ("Remove?", "Does it earn its place on most visits? Delete text that repeats a label, a heading that repeats the values under it, and a status that is obvious."),
    ("Merge?", "Is the same fact shown twice, like a title and a name field, or a group label and its rows? Keep one."),
    ("Disclose later?", "Is it needed only sometimes? Show a calm summary first. Put editing behind Edit, a long list behind one row that opens it, and rare or risky actions (delete, export, test) in a menu, not on every row."),
    ("Shrink?", "Could a well known icon (add, share, play, more) replace a word? Could a value sit at the end of its row instead of on its own row?"),
];

pub const KEEP: &str = "Keep these even when they look like clutter: the main action, close, stop and cancel buttons, error and safety messages, anything the task asked for, a text label or accessible name on every control (an icon button too), a visible keyboard focus, and readable text size and contrast.";

pub fn steps() -> Vec<String> {
    STEPS.iter().enumerate().map(|(index, (name, ask))| format!("{}. {name} {ask}", index + 1)).collect()
}

pub fn system() -> String {
    let mut lines = vec!["Review this screen with the declutter check. Go through every visible word, control, colour and box, top to bottom, and ask these four questions in this order:".to_string()];
    lines.extend(steps());
    lines.push(KEEP.to_string());
    lines.push("For each question, name the items it applies to and what to change, or say none. Then name what you kept on purpose from the keep list.".into());
    lines.push(format!("End with one line: {COUNT_WORD} <number of changes you suggest> ({COUNT_WORD} 0 when nothing more can go)."));
    lines.join("\n")
}

pub fn rubric_system(rubric: Option<&str>) -> String {
    match rubric {
        Some(RUBRIC) => system(),
        _ => String::new(),
    }
}

pub fn system_for(base: &str, rubric: Option<&str>) -> String {
    [base.to_string(), rubric_system(rubric)].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n\n")
}

pub fn changes_of(answer: &str) -> Option<u32> {
    answer.lines().rev().find_map(|line| {
        let at = line.find(COUNT_WORD)?;
        let rest = line[at + COUNT_WORD.len()..].trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    })
}

pub fn keep_count(full: &str, clipped: String) -> String {
    match changes_of(full) {
        Some(count) if changes_of(&clipped) != Some(count) => format!("{clipped}\n{COUNT_WORD} {count}"),
        _ => clipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_check_asks_four_questions_in_order_and_keeps_safety() {
        let text = system();
        let order: Vec<usize> = ["1. Remove?", "2. Merge?", "3. Disclose later?", "4. Shrink?"].iter().map(|s| text.find(s).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
        assert!(text.contains("accessible name on every control"));
        assert!(text.ends_with("(DECLUTTER: 0 when nothing more can go)."));
    }

    #[test]
    fn only_the_declutter_rubric_adds_to_the_system_text() {
        assert_eq!(system_for("Look.", None), "Look.");
        assert_eq!(system_for("Look.", Some("other")), "Look.");
        let both = system_for("Look.", Some(RUBRIC));
        assert!(both.starts_with("Look.\n\nReview this screen"));
    }

    #[test]
    fn reads_the_change_count_from_the_last_line() {
        assert_eq!(changes_of("Remove? none.\nDECLUTTER: 3"), Some(3));
        assert_eq!(changes_of("DECLUTTER: 2\nmore text\nDECLUTTER: 0"), Some(0));
        assert_eq!(changes_of("no count here"), None);
        assert_eq!(changes_of("DECLUTTER: many"), None);
    }

    #[test]
    fn a_cut_answer_keeps_its_change_count() {
        assert_eq!(keep_count("long answer\nDECLUTTER: 4", "long".into()), "long\nDECLUTTER: 4");
        assert_eq!(keep_count("short\nDECLUTTER: 2", "short\nDECLUTTER: 2".into()), "short\nDECLUTTER: 2");
        assert_eq!(keep_count("no count", "no".into()), "no");
    }
}
