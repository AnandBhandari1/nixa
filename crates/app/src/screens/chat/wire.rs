use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Typed(String),
    Stop,
    Clean,
    KeywordClear(&'static str),
    Listen(&'static str),
    Mic(bool),
    Voice(bool),
    PhoneVoice(bool),
    Confirm {
        id: String,
        approve: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wire {
    Ws(Value),
    Request {
        kind: &'static str,
        fields: Value,
    },
    Post { path: &'static str, body: Value },
}

pub fn wire(cmd: &Command) -> Wire {
    match cmd {
        Command::Typed(text) => Wire::Ws(json!({"type": "typed", "text": text})),
        Command::Stop => Wire::Ws(json!({"type": "interrupt"})),
        Command::Clean => Wire::Post {
            path: "/chat/clean",
            body: json!({}),
        },
        Command::KeywordClear(what) => Wire::Request {
            kind: "keyword_clear",
            fields: json!({"what": what}),
        },
        Command::Listen(mode) => Wire::Ws(json!({"type": "set_listen", "mode": mode})),
        Command::Mic(on) => Wire::Ws(json!({"type": "set_mic", "on": on})),
        Command::Voice(on) => Wire::Ws(json!({"type": "set_voice", "on": on})),
        Command::PhoneVoice(on) => Wire::Ws(crate::screens::phone_chip::info::set_voice(*on)),
        Command::Confirm { id, approve } => {
            Wire::Ws(json!({"type": "confirm", "id": id, "approve": approve}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_commands_go_over_the_socket() {
        assert_eq!(
            wire(&Command::Typed("hi".into())),
            Wire::Ws(json!({"type": "typed", "text": "hi"}))
        );
        assert_eq!(wire(&Command::Stop), Wire::Ws(json!({"type": "interrupt"})));
        assert_eq!(
            wire(&Command::Mic(true)),
            Wire::Ws(json!({"type": "set_mic", "on": true}))
        );
        assert_eq!(
            wire(&Command::Listen("hotkey")),
            Wire::Ws(json!({"type": "set_listen", "mode": "hotkey"}))
        );
        assert_eq!(
            wire(&Command::KeywordClear("hold")),
            Wire::Request {
                kind: "keyword_clear",
                fields: json!({"what": "hold"})
            }
        );
        assert_eq!(
            wire(&Command::Confirm {
                id: "c1".into(),
                approve: false
            }),
            Wire::Ws(json!({"type": "confirm", "id": "c1", "approve": false}))
        );
    }

    #[test]
    fn clean_uses_the_brain_route() {
        assert_eq!(
            wire(&Command::Clean),
            Wire::Post {
                path: "/chat/clean",
                body: json!({})
            }
        );
    }
}
