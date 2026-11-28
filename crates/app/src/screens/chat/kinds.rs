use super::model::Item;

pub const FALLBACK: &str = "Message";
pub const EVENT_MARK: &str = "Event: ";

pub fn kind_label(kind: &str, assistant: &str) -> String {
    if kind == "nixa" && !assistant.is_empty() {
        return assistant.to_string();
    }
    match kind {
        "typed" => "Typed",
        "heard" => "Heard",
        "nixa" => "Nixa",
        "agent" => "Agent",
        "sent" => "Sent",
        "note" => "Note",
        "question" => "Question",
        "permission" => "Permission",
        "plan" => "Plan",
        "confirm" => "Confirm",
        "clean" => "Clean",
        _ => FALLBACK,
    }
    .to_string()
}

pub fn is_sent(item: &Item) -> bool {
    item.who == "agent" && (item.to.is_some() || item.kind.as_deref() == Some("sent"))
}

pub fn is_brain_event(item: &Item) -> bool {
    let spoken = item.who == "info" || (item.who == "user" && !item.typed);
    spoken && item.text.trim_start().starts_with(EVENT_MARK)
}

pub fn item_kind(item: &Item) -> &'static str {
    match item.who.as_str() {
        "user" if item.typed => "typed",
        "user" => "heard",
        "nixa" => "nixa",
        "agent" if is_sent(item) => "sent",
        "agent" => "agent",
        "info" => "note",
        _ => "",
    }
}

pub fn item_label(item: &Item, assistant: &str) -> String {
    kind_label(item_kind(item), assistant)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(who: &str) -> Item {
        Item {
            who: who.into(),
            ..Item::default()
        }
    }

    #[test]
    fn every_who_has_its_kind() {
        let mut typed = item("user");
        typed.typed = true;
        assert_eq!(item_kind(&typed), "typed");
        assert_eq!(item_kind(&item("user")), "heard");
        assert_eq!(item_kind(&item("nixa")), "nixa");
        assert_eq!(item_kind(&item("agent")), "agent");
        assert_eq!(item_kind(&item("info")), "note");
        let mut sent = item("agent");
        sent.to = Some("m3".into());
        assert_eq!(item_kind(&sent), "sent");
        let mut marked = item("agent");
        marked.kind = Some("sent".into());
        assert_eq!(item_kind(&marked), "sent");
    }

    #[test]
    fn brain_event_lines_are_found() {
        let mut note = item("info");
        note.text = "Event: Terminal m8 (tests) reported an error. Read its screen.".into();
        assert!(is_brain_event(&note));
        let mut heard = item("user");
        heard.text = "  Event: Prompt p1: An agent asks".into();
        assert!(is_brain_event(&heard));
        let mut typed = item("user");
        typed.typed = true;
        typed.text = "Event: my own words".into();
        assert!(!is_brain_event(&typed));
        let mut reply = item("nixa");
        reply.text = "Event: said by Nixa".into();
        assert!(!is_brain_event(&reply));
        let mut plain = item("info");
        plain.text = "The mic did not start.".into();
        assert!(!is_brain_event(&plain));
    }

    #[test]
    fn unknown_kinds_get_the_fallback() {
        assert_eq!(item_label(&item("robot"), "Nixa"), FALLBACK);
        assert_eq!(kind_label("later", "Nixa"), FALLBACK);
    }

    #[test]
    fn the_nixa_row_uses_the_assistant_name() {
        assert_eq!(item_label(&item("nixa"), "Juno"), "Juno");
        assert_eq!(kind_label("nixa", ""), "Nixa");
        assert_eq!(kind_label("clean", "Juno"), "Clean");
    }
}
