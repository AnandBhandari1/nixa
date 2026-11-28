use anyhow::{Result, anyhow, bail};
use crate::{Reply, Request, paths};
use std::path::Path;
use std::time::Duration;
use std::collections::HashMap;
use std::process::Command;

pub const PS: &str = "/bin/ps";
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    User,
    Agent(String),
}

impl Caller {
    pub fn terminal(&self) -> Option<&str> {
        match self {
            Caller::User => None,
            Caller::Agent(id) => Some(id.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub id: String,
    pub pid: Option<u32>,
    pub running: bool,
}

pub fn parse_parents(stdout: &str) -> HashMap<u32, u32> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let parent = parts.next()?.parse().ok()?;
            parts.next().is_none().then_some((pid, parent))
        })
        .collect()
}

pub fn chain_of(parents: &HashMap<u32, u32>, pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = Some(pid);
    while let Some(now) = current {
        if now == 0 || chain.contains(&now) || chain.len() >= MAX_DEPTH {
            break;
        }
        chain.push(now);
        current = parents.get(&now).copied();
    }
    chain
}

fn parents() -> Option<HashMap<u32, u32>> {
    let out = Command::new(PS).args(["-A", "-o", "pid=,ppid="]).output().ok()?;
    out.status.success().then(|| parse_parents(&String::from_utf8_lossy(&out.stdout)))
}

fn live_terms(socket: &Path) -> Option<Vec<Live>> {
    let mut client = crate::connect(socket).ok()?;
    client.set_timeout(Some(Duration::from_secs(15))).ok()?;
    match client.call(&Request::List { parent: None, label: None }) {
        Ok(Reply::Terms { terms }) => Some(terms.into_iter().map(|t| Live { id: t.id.to_string(), pid: t.pid, running: t.running }).collect()),
        _ => None,
    }
}

fn holds(term: &Live, chain: &[u32]) -> bool {
    term.running && term.pid.is_some_and(|pid| pid > 1 && chain.contains(&pid))
}

pub fn decide(claimed: Option<&str>, terms: Option<&[Live]>, chain: Option<&[u32]>) -> Result<Caller> {
    let Some(id) = claimed else {
        let Some(terms) = terms.filter(|all| all.iter().any(|t| t.running && t.pid.is_some())) else {
            return Ok(Caller::User);
        };
        let chain = chain.ok_or_else(|| anyhow!("Nixa cannot read the process list to check who calls. Nothing was changed."))?;
        return Ok(match terms.iter().find(|t| holds(t, chain)) {
            Some(term) => Caller::Agent(term.id.clone()),
            None => Caller::User,
        });
    };
    let terms = terms.ok_or_else(|| anyhow!("Nixa cannot check that this call comes from terminal {id}: nixa-engine does not answer. Nothing was changed."))?;
    let chain = chain.ok_or_else(|| anyhow!("Nixa cannot read the process list to check terminal {id}. Nothing was changed."))?;
    let Some(term) = terms.iter().find(|t| t.id == id) else {
        bail!("Terminal {id} (NIXA_TERM_ID) is not an open Nixa terminal. Nothing was changed.");
    };
    if !holds(term, chain) {
        bail!("This call does not come from a program inside terminal {id}. Run nixa task in that terminal. Nothing was changed.");
    }
    Ok(Caller::Agent(id.to_string()))
}

pub fn who(socket: &Path) -> Result<Caller> {
    let claimed = std::env::var(paths::ENV_TERM_ID).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let terms = live_terms(socket);
    let busy = terms.as_ref().is_some_and(|all| all.iter().any(|t| t.running && t.pid.is_some()));
    let chain = if claimed.is_some() || busy { parents().map(|table| chain_of(&table, std::process::id())) } else { None };
    decide(claimed.as_deref(), terms.as_deref(), chain.as_deref())
}

pub fn user_only(caller: &Caller, what: &str) -> Result<()> {
    match caller {
        Caller::User => Ok(()),
        Caller::Agent(id) => bail!(
            "Only the user can {what}, from the Nixa window or a shell outside Nixa terminals. Terminal {id} cannot. Nothing was changed. Do not wait for the user: ask Nixa to decide it with nixa deep ask, then go on with your own work."
        ),
    }
}

pub fn user_or_auto(caller: &Caller, what: &str, auto: bool) -> Result<()> {
    if auto {
        return Ok(());
    }
    user_only(caller, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(id: &str, pid: u32) -> Live {
        Live { id: id.into(), pid: Some(pid), running: true }
    }

    #[test]
    fn parents_and_chain() {
        let table = parse_parents(" 10 1\n20 10\n30 20\nbad line\n40 30 extra\n");
        assert_eq!(table.len(), 3);
        assert_eq!(chain_of(&table, 30), vec![30, 20, 10, 1]);
        assert_eq!(chain_of(&table, 99), vec![99]);
        let looped: HashMap<u32, u32> = [(5, 6), (6, 5)].into_iter().collect();
        assert_eq!(chain_of(&looped, 5), vec![5, 6]);
    }

    #[test]
    fn named_terminal_must_hold_the_caller() {
        let terms = vec![live("m1", 20), live("m2", 70)];
        let chain: Vec<u32> = vec![30, 20, 10, 1];
        let (terms, chain) = (terms.as_slice(), chain.as_slice());
        assert_eq!(decide(Some("m1"), Some(terms), Some(chain)).unwrap(), Caller::Agent("m1".into()));
        let err = decide(Some("m2"), Some(terms), Some(chain)).unwrap_err().to_string();
        assert!(err.contains("does not come from a program inside terminal m2"), "{err}");
        let err = decide(Some("m9"), Some(terms), Some(chain)).unwrap_err().to_string();
        assert!(err.contains("not an open Nixa terminal"), "{err}");
        assert!(decide(Some("m1"), None, Some(chain)).unwrap_err().to_string().contains("does not answer"));
        assert!(decide(Some("m1"), Some(terms), None).is_err());
        let stopped = vec![Live { id: "m1".into(), pid: Some(20), running: false }];
        assert!(decide(Some("m1"), Some(stopped.as_slice()), Some(chain)).is_err());
        let root = vec![live("m1", 1)];
        assert!(decide(Some("m1"), Some(root.as_slice()), Some(chain)).is_err());
    }

    #[test]
    fn no_name_means_user_unless_inside_a_terminal() {
        let chain: Vec<u32> = vec![30, 20, 10, 1];
        let none: Vec<Live> = Vec::new();
        assert_eq!(decide(None, None, None).unwrap(), Caller::User);
        assert_eq!(decide(None, Some(none.as_slice()), None).unwrap(), Caller::User);
        let outside = vec![live("m2", 70)];
        assert_eq!(decide(None, Some(outside.as_slice()), Some(chain.as_slice())).unwrap(), Caller::User);
        assert!(decide(None, Some(outside.as_slice()), None).is_err());
        let inside = vec![live("m2", 70), live("m1", 20)];
        assert_eq!(decide(None, Some(inside.as_slice()), Some(chain.as_slice())).unwrap(), Caller::Agent("m1".into()));
    }

    #[test]
    fn only_users_pass_user_only() {
        assert!(user_only(&Caller::User, "set a step plan").is_ok());
        let err = user_only(&Caller::Agent("m3".into()), "set a step plan").unwrap_err().to_string();
        assert!(err.contains("Only the user can set a step plan") && err.contains("Terminal m3 cannot"), "{err}");
        assert!(err.contains("nixa deep ask"), "{err}");
    }

    #[test]
    fn automatic_lets_agents_through() {
        let agent = Caller::Agent("m3".into());
        assert!(user_or_auto(&agent, "run a step plan", true).is_ok());
        assert!(user_or_auto(&agent, "run a step plan", false).is_err());
        assert!(user_or_auto(&Caller::User, "run a step plan", false).is_ok());
    }
}
