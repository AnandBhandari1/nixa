use crate::app::lock;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
struct Marks {
    next: u64,
    done: u64,
    finished: BTreeSet<u64>,
}

#[derive(Default)]
pub struct Queue {
    marks: Mutex<Marks>,
    turn: Notify,
}

pub struct Turn {
    queue: Arc<Queue>,
    id: u64,
}

pub fn busy(queue: &Queue) -> bool {
    let marks = lock(&queue.marks);
    marks.next != marks.done
}

pub fn take(queue: &Arc<Queue>) -> Turn {
    let mut marks = lock(&queue.marks);
    let id = marks.next;
    marks.next += 1;
    Turn { queue: queue.clone(), id }
}

pub async fn wait(turn: &Turn) {
    loop {
        let notified = turn.queue.turn.notified();
        if lock(&turn.queue.marks).done >= turn.id {
            return;
        }
        notified.await;
    }
}

fn finish(queue: &Queue, id: u64) {
    let mut marks = lock(&queue.marks);
    marks.finished.insert(id);
    loop {
        let done = marks.done;
        if !marks.finished.remove(&done) {
            break;
        }
        marks.done += 1;
    }
    drop(marks);
    queue.turn.notify_waiters();
}

impl Drop for Turn {
    fn drop(&mut self) {
        finish(&self.queue, self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_slow_first_turn_still_goes_first() {
        let queue = Arc::new(Queue::default());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let first = take(&queue);
        let second = take(&queue);
        assert!(busy(&queue));
        let slow = {
            let seen = seen.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(80)).await;
                wait(&first).await;
                lock(&seen).push(1);
            })
        };
        let fast = {
            let seen = seen.clone();
            tokio::spawn(async move {
                wait(&second).await;
                lock(&seen).push(2);
            })
        };
        fast.await.unwrap_or_default();
        slow.await.unwrap_or_default();
        assert_eq!(*lock(&seen), vec![1, 2]);
        assert!(!busy(&queue));
    }

    #[tokio::test]
    async fn a_lost_turn_does_not_block_the_rest() {
        let queue = Arc::new(Queue::default());
        let first = take(&queue);
        let second = take(&queue);
        let third = take(&queue);
        drop(second);
        assert!(busy(&queue));
        drop(first);
        tokio::time::timeout(Duration::from_secs(1), wait(&third)).await.unwrap_or_default();
        assert_eq!(lock(&queue.marks).done, 2);
        drop(third);
        assert!(!busy(&queue));
    }
}
