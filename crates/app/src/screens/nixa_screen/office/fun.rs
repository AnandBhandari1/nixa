use super::motion;
use super::npc;
use super::plan::{self, Plan, Role};
use super::soap::{self, Gate};
use super::talk::{self, Stage, Talk};
use crate::screens::nixa_screen::map::{Agent, St};
use serde_json::{Value, json};

pub const FUN_KIND: &str = "office_fun";
pub const FUN_EVENT: &str = "office_fun_line";
pub const GATE_KIND: &str = "office_fun_gate";
pub const BANTER_KIND: &str = "office_banter";
pub const BANTER_EVENT: &str = "office_banter_line";
pub const GAP_MIN: u64 = 40_000;
pub const GAP_MAX: u64 = 90_000;
pub const BANTER_WAIT_MS: u64 = 10_000;
pub const GATE_EVERY_MS: u64 = 30_000;
pub const LINE_MAX: usize = 110;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fun {
    pub next: u64,
    pub gate: Option<(bool, bool)>,
    pub gate_at: u64,
    pub banter_at: u64,
    pub banter_asked: Option<u64>,
}

pub fn on(g: Gate) -> bool {
    g.office && g.open && g.on && g.up
}

pub fn gap(seed: u64) -> u64 {
    GAP_MIN + motion::hash(seed, 41) % (GAP_MAX - GAP_MIN + 1)
}

fn name_of(plan: &Plan, id: &str) -> String {
    plan::find(plan, id).map(|p| p.short.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| id.to_string())
}

fn known(plan: &Plan, agents: &[Agent], from: &str, to: &str) -> Vec<String> {
    let mut out: Vec<String> = plan.people.iter().map(|p| p.short.clone()).filter(|s| !s.is_empty()).collect();
    for id in [from, to] {
        let title = plan::find(plan, id)
            .and_then(|p| agents.iter().find(|a| a.id == p.id))
            .map(|a| soap::cut(&a.title, soap::TASK_MAX))
            .unwrap_or_default();
        if !title.is_empty() && !out.contains(&title) {
            out.push(title);
        }
    }
    out
}

pub fn asks(stage: &mut Stage, plan: &Plan, agents: &[Agent], g: Gate, now: u64) -> Vec<(u64, Value)> {
    let mut out = Vec::new();
    for t in stage.queue.iter_mut().filter(|t| t.ask == 0 && !t.ready && !t.banter) {
        stage.fun.next += 1;
        t.ask = stage.fun.next;
        t.asked = now;
        out.push((
            t.ask,
            json!({"id": t.ask, "from_name": name_of(plan, &t.from), "to_name": name_of(plan, &t.to),
                "first": t.first, "second": t.second, "known": known(plan, agents, &t.from, &t.to),
                "on": g.on, "office": g.office && g.open}),
        ));
    }
    out
}

pub fn unask(stage: &mut Stage, id: u64) {
    for t in stage.queue.iter_mut().filter(|t| t.ask == id) {
        t.ask = 0;
        t.ready = true;
    }
}

fn line(msg: &Value, key: &str) -> Option<String> {
    msg.get(key).and_then(Value::as_str).map(|t| soap::cut(t, LINE_MAX)).filter(|t| !t.is_empty())
}

pub fn apply(stage: &mut Stage, msg: &Value) -> bool {
    let Some(id) = msg.get("id").and_then(Value::as_u64).filter(|id| *id > 0) else {
        return false;
    };
    let (first, second) = (line(msg, "first"), line(msg, "second"));
    if let Some(t) = stage.queue.iter_mut().find(|t| t.ask == id) {
        if let Some(f) = first {
            t.first = f;
        }
        if let Some(s) = second {
            t.second = s;
        }
        t.ready = true;
        return true;
    }
    if let Some(p) = stage.now.as_mut().filter(|p| p.talk.ask == id) {
        if let Some(f) = first.filter(|_| p.said == 0) {
            p.say1 = talk::say1_ms(&f);
            p.talk.first = f;
        }
        if let Some(s) = second.filter(|_| p.said < 2) {
            p.say2 = talk::say2_ms(&s);
            p.talk.second = s;
        }
        p.talk.ready = true;
        return true;
    }
    false
}

pub fn gate_msg(fun: &mut Fun, g: Gate, now: u64) -> Option<Value> {
    let state = (g.on, g.office && g.open);
    let stale = now.saturating_sub(fun.gate_at) >= GATE_EVERY_MS;
    if fun.gate == Some(state) && !stale {
        return None;
    }
    fun.gate = Some(state);
    fun.gate_at = now;
    Some(json!({"on": state.0, "office": state.1}))
}

pub fn drop_banter(stage: &mut Stage) {
    stage.queue.retain(|t| !t.banter);
    if stage.now.as_ref().is_some_and(|p| p.talk.banter && p.said == 0) {
        stage.now = None;
    }
}

pub fn schedule(fun: &mut Fun, g: Gate, now: u64) {
    if !on(g) || g.idle {
        fun.banter_at = 0;
        fun.banter_asked = None;
    } else if fun.banter_at == 0 {
        fun.banter_at = now + gap(now);
    }
}

pub fn banter_due(fun: &Fun, g: Gate, show: bool, now: u64) -> bool {
    on(g)
        && !g.idle
        && !g.talk
        && !show
        && fun.banter_at > 0
        && now >= fun.banter_at
        && fun.banter_asked.is_none_or(|a| now.saturating_sub(a) >= BANTER_WAIT_MS)
}

pub fn banter_pick(plan: &Plan, agents: &[Agent], g: Gate, now: u64) -> Option<Value> {
    let free: Vec<&plan::Person> = plan.people.iter().filter(|p| npc::is_free(p)).collect();
    if free.len() < 2 {
        return None;
    }
    let work: Vec<(&plan::Person, String)> = plan
        .people
        .iter()
        .filter(|p| p.role == Role::Staff && p.status == St::Working)
        .filter_map(|p| {
            let a = agents.iter().find(|a| a.id == p.id)?;
            let task = soap::cut(&a.title, soap::TASK_MAX);
            (!task.is_empty()).then_some((p, task))
        })
        .collect();
    if work.is_empty() {
        return None;
    }
    let seed = now / 1000;
    let (owner, task) = &work[(motion::hash(seed, 1) % work.len() as u64) as usize];
    let i = (motion::hash(seed, 2) % free.len() as u64) as usize;
    let j = (i + 1 + (motion::hash(seed, 3) % (free.len() as u64 - 1)) as usize) % free.len();
    let (a, b) = (free[i], free[j]);
    Some(json!({"who": a.id, "name": a.short, "to": b.id, "to_name": b.short, "owner": owner.short, "task": task,
        "on": g.on, "office": g.office && g.open}))
}

pub fn banter_talk(msg: &Value, plan: &Plan, now: u64) -> Option<Talk> {
    let id = |k: &str| msg.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let (who, to) = (id("who")?, id("to")?);
    let free = |x: &str| plan.people.iter().any(|p| p.id == x && npc::is_free(p));
    if who == to || !free(who) || !free(to) {
        return None;
    }
    Some(Talk {
        from: who.to_string(),
        to: to.to_string(),
        first: line(msg, "first")?,
        second: line(msg, "second")?,
        given: true,
        at: now,
        ask: 0,
        asked: 0,
        ready: true,
        banter: true,
    })
}
