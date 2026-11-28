use super::model::Work;

pub struct Keyword {
    pub id: &'static str,
    pub label: String,
    pub tip: String,
    pub what: &'static str,
}

pub fn keyword(work: &Work, name: &str) -> Option<Keyword> {
    if work.hold {
        return Some(Keyword {
            id: "nixa-hold",
            label: "Hold".into(),
            tip: "Hold is on: nothing runs by itself, no agent question is answered and no check runs. Click to end the hold, or say go on.".into(),
            what: "hold",
        });
    }
    let topic = work.topic.as_ref()?;
    let word = if topic.kind == "plan" {
        "Plan"
    } else {
        "Discuss"
    };
    let short: String = topic.text.chars().take(40).collect();
    let label = if short.is_empty() {
        word.to_string()
    } else {
        format!("{word}: {short}")
    };
    Some(Keyword {
        id: "nixa-topic",
        label,
        tip: format!("{word} is on: {name} starts nobody until you say assign. Click to end it."),
        what: "topic",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::chat::model::Topic;

    #[test]
    fn hold_wins_over_topic() {
        let mut work = Work {
            topic: Some(Topic {
                kind: "plan".into(),
                text: "x".into(),
            }),
            ..Work::default()
        };
        assert_eq!(keyword(&work, "Nixa").unwrap().label, "Plan: x");
        work.hold = true;
        let k = keyword(&work, "Nixa").unwrap();
        assert_eq!((k.id, k.what), ("nixa-hold", "hold"));
    }

    #[test]
    fn topic_text_is_cut_at_40() {
        let work = Work {
            topic: Some(Topic {
                kind: "talk".into(),
                text: "a".repeat(60),
            }),
            ..Work::default()
        };
        let k = keyword(&work, "Nixa").unwrap();
        assert_eq!(k.label, format!("Discuss: {}", "a".repeat(40)));
        assert!(k.tip.starts_with("Discuss is on: Nixa"));
        let bare = Work {
            topic: Some(Topic::default()),
            ..Work::default()
        };
        assert_eq!(keyword(&bare, "Nixa").unwrap().label, "Discuss");
        assert!(keyword(&Work::default(), "Nixa").is_none());
    }
}
