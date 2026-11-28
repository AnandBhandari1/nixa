use super::{begin_read, stop_read};
use crate::app::{App, lock};
use crate::tools::Args;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(super) struct Reads {
    next: u64,
    pending: HashMap<u64, Read>,
}

#[derive(Default)]
struct Read {
    stop: CancellationToken,
    done: CancellationToken,
}

impl Reads {
    pub(super) fn start(&mut self) -> (u64, CancellationToken) {
        self.next += 1;
        let read = Read::default();
        let stop = read.stop.clone();
        self.pending.insert(self.next, read);
        (self.next, stop)
    }

    pub(super) fn stop_token(&self, id: u64) -> Option<CancellationToken> {
        self.pending.get(&id).map(|read| read.stop.clone())
    }

    pub(super) fn stop(&self, id: Option<u64>) {
        for (key, read) in &self.pending {
            if id.is_none_or(|id| id == *key) {
                read.stop.cancel();
            }
        }
    }

    pub(super) fn finish(&mut self, id: u64) {
        if let Some(read) = self.pending.remove(&id) {
            read.done.cancel();
        }
    }
}

pub fn begin(app: &Arc<App>, text: &str, task: i64) -> Value {
    let read = begin_read(app, text, task);
    json!({
        "spoken": read.is_some(),
        "held": read.is_some_and(|(_, held)| held),
        "read": read.filter(|(_, held)| *held).map(|(mark, _)| mark.read),
        "read_id": read.map(|(mark, _)| mark.read),
    })
}

pub async fn wait(app: &App, args: &Args) -> Result<Value, String> {
    let id = read_id(args)?.ok_or("A read ID is needed.")?;
    let done = lock(&app.talk.hold)
        .reads
        .pending
        .get(&id)
        .map(|read| read.done.clone());
    if let Some(done) = done {
        done.cancelled().await;
    }
    Ok(json!({"read": id}))
}

pub fn stop(app: &App, args: &Args) -> Result<Value, String> {
    let dropped = stop_read(app, read_id(args)?);
    Ok(json!({"stopped": true, "dropped": dropped}))
}

fn read_id(args: &Args) -> Result<Option<u64>, String> {
    args.get("read")
        .map(|v| {
            v.as_u64()
                .filter(|id| *id != 0)
                .ok_or_else(|| "Invalid read ID.".into())
        })
        .transpose()
}
