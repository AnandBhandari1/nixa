use super::map::{DEEP, Flow, NIXA};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const CUE_WINDOW_MS: u64 = 300;
pub const KIND: &str = "universe_meteor";

#[derive(Default)]
pub struct Cues {
    seen: BTreeSet<Flow>,
}

pub fn visible(who: &str, shown: &[String]) -> bool {
    who == NIXA || who == DEEP || shown.iter().any(|s| s == who)
}

pub fn node_of(name: &str) -> &str {
    match name.trim() {
        "core" | "nixa" | "Nixa" => NIXA,
        "deep" | "deep_brain" | "deep-brain" => DEEP,
        other => other,
    }
}

pub fn cue(cues: &mut Cues, flows: &[Flow], now: u64, shown: &[String]) -> Option<Value> {
    let mut out = None;
    for flow in flows {
        if !cues.seen.insert(flow.clone()) {
            continue;
        }
        if flow.at > now
            || now - flow.at > CUE_WINDOW_MS
            || !visible(&flow.from, shown)
            || !visible(&flow.to, shown)
        {
            continue;
        }
        if out.is_none() {
            let direction = if flow.to == NIXA {
                "incoming"
            } else {
                "outgoing"
            };
            out = Some(json!({"direction": direction, "at": flow.at}));
        }
    }
    cues.seen.retain(|f| flows.contains(f));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(from: &str, to: &str, at: u64) -> Flow {
        Flow {
            from: from.into(),
            to: to.into(),
            at,
        }
    }

    #[test]
    fn a_meteor_cue_is_sent_once_for_a_fresh_visible_flow() {
        let mut cues = Cues::default();
        let shown = vec!["m2".to_string()];
        let flows = vec![flow("m2", NIXA, 1000)];
        assert_eq!(
            cue(&mut cues, &flows, 1050, &shown),
            Some(json!({"direction": "incoming", "at": 1000}))
        );
        assert_eq!(cue(&mut cues, &flows, 1060, &shown), None);
        let hidden = vec![flow(NIXA, "m7", 5000)];
        assert_eq!(cue(&mut cues, &hidden, 5000, &shown), None);
        let stale = vec![flow(NIXA, "m2", 9000)];
        assert_eq!(cue(&mut cues, &stale, 9000 + CUE_WINDOW_MS + 1, &shown), None);
        let out = vec![flow(NIXA, DEEP, 12000)];
        assert_eq!(
            cue(&mut cues, &out, 12000, &shown),
            Some(json!({"direction": "outgoing", "at": 12000}))
        );
    }

    #[test]
    fn names_map_to_nodes() {
        assert_eq!(node_of("core"), NIXA);
        assert_eq!(node_of("deep_brain"), DEEP);
        assert_eq!(node_of("m4"), "m4");
    }
}
