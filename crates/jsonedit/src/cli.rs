pub enum Action {
    Help,
    Run { tree: String, brain: Option<String> },
}

pub const USAGE: &str = "Usage: nixa-json <tree-name> [--brain <host:port>]";

pub fn good_tree_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 60 {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut args = args.iter();
    let mut name: Option<String> = None;
    let mut brain: Option<String> = None;
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            return Ok(Action::Help);
        }
        if arg == "--brain" {
            brain = Some(
                args.next()
                    .cloned()
                    .ok_or_else(|| "--brain needs host:port.".to_string())?,
            );
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--brain=") {
            brain = Some(rest.to_string());
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("Unknown option: {arg}"));
        }
        if name.is_some() {
            return Err("Give one tree name, for example: nixa-json watch-agents".to_string());
        }
        name = Some(arg.clone());
    }
    let tree = name.ok_or_else(|| USAGE.to_string())?;
    if !good_tree_name(&tree) {
        return Err(format!(
            "Invalid tree name \"{tree}\": use lowercase letters, numbers and dashes."
        ));
    }
    Ok(Action::Run {
        tree,
        brain: brain.filter(|b| !b.trim().is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn run_of(list: &[&str]) -> (String, Option<String>) {
        match parse(&args(list)) {
            Ok(Action::Run { tree, brain }) => (tree, brain),
            Ok(Action::Help) => panic!("help for {list:?}"),
            Err(err) => panic!("error for {list:?}: {err}"),
        }
    }

    #[test]
    fn names_and_brain_option() {
        assert_eq!(
            run_of(&["watch-agents"]),
            ("watch-agents".to_string(), None)
        );
        assert_eq!(
            run_of(&["t1", "--brain", "127.0.0.1:7425"]),
            ("t1".to_string(), Some("127.0.0.1:7425".to_string()))
        );
        assert_eq!(
            run_of(&["--brain=localhost:7421", "a-b"]),
            ("a-b".to_string(), Some("localhost:7421".to_string()))
        );
        assert_eq!(run_of(&["x", "--brain="]), ("x".to_string(), None));
    }

    #[test]
    fn help_and_errors() {
        assert!(matches!(parse(&args(&["-h"])), Ok(Action::Help)));
        assert!(matches!(
            parse(&args(&["tree", "--help"])),
            Ok(Action::Help)
        ));
        assert_eq!(parse(&args(&[])).err().unwrap(), USAGE);
        assert!(
            parse(&args(&["--brain"]))
                .err()
                .unwrap()
                .contains("needs host:port")
        );
        assert!(
            parse(&args(&["--nixa-task", "x"]))
                .err()
                .unwrap()
                .contains("Unknown option")
        );
        assert!(
            parse(&args(&["a", "b"]))
                .err()
                .unwrap()
                .contains("one tree name")
        );
        assert!(
            parse(&args(&["Bad"]))
                .err()
                .unwrap()
                .contains("Invalid tree name")
        );
    }

    #[test]
    fn tree_name_rules() {
        assert!(good_tree_name("watch-agents"));
        assert!(good_tree_name("0abc"));
        assert!(good_tree_name(&"a".repeat(60)));
        assert!(!good_tree_name(&"a".repeat(61)));
        assert!(!good_tree_name(""));
        assert!(!good_tree_name("-a"));
        assert!(!good_tree_name("a_b"));
        assert!(!good_tree_name("a/b"));
        assert!(!good_tree_name("../x"));
    }
}
