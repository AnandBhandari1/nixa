use std::collections::HashMap;

pub const REVIEW_RETRY_MS: [u64; 3] = [60_000, 120_000, 240_000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StallKind {
    Checks,
    Same,
    Review,
}

impl StallKind {
    pub const ALL: [StallKind; 3] = [StallKind::Checks, StallKind::Same, StallKind::Review];

    pub fn limit(self) -> u32 {
        match self {
            StallKind::Checks => 2,
            StallKind::Same => 1,
            StallKind::Review => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverInput {
    pub rounds: u32,
    pub max_rounds: u32,
    pub cap_allows: bool,
}

pub fn can_recover(input: &RecoverInput) -> bool {
    input.rounds < input.max_rounds && input.cap_allows
}

#[derive(Debug, Default)]
pub struct Stalls {
    counts: HashMap<(StallKind, String), u32>,
}

impl Stalls {
    pub fn take(&mut self, kind: StallKind, terminal: &str) -> Option<u32> {
        let used = self.used(kind, terminal);
        if used >= kind.limit() {
            return None;
        }
        self.counts.insert((kind, terminal.to_string()), used + 1);
        Some(used + 1)
    }

    pub fn used(&self, kind: StallKind, terminal: &str) -> u32 {
        self.counts.get(&(kind, terminal.to_string())).copied().unwrap_or(0)
    }

    pub fn clear(&mut self, terminal: &str) {
        for kind in StallKind::ALL {
            self.counts.remove(&(kind, terminal.to_string()));
        }
    }
}

pub fn review_delay(attempt: u32) -> Option<u64> {
    let index = usize::try_from(attempt).ok()?.checked_sub(1)?;
    REVIEW_RETRY_MS.get(index).copied()
}

pub fn plan_review(stalls: &mut Stalls, terminal: &str) -> Option<(u32, u64)> {
    let attempt = stalls.take(StallKind::Review, terminal)?;
    review_delay(attempt).map(|delay| (attempt, delay))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_kind_and_terminal() {
        let mut stalls = Stalls::default();
        assert_eq!(stalls.take(StallKind::Checks, "m1"), Some(1));
        assert_eq!(stalls.take(StallKind::Checks, "m1"), Some(2));
        assert_eq!(stalls.take(StallKind::Checks, "m1"), None);
        assert_eq!(stalls.take(StallKind::Checks, "m2"), Some(1));
        assert_eq!(stalls.take(StallKind::Same, "m1"), Some(1));
        assert_eq!(stalls.take(StallKind::Same, "m1"), None);
        stalls.clear("m1");
        assert_eq!(stalls.take(StallKind::Checks, "m1"), Some(1));
        assert_eq!(stalls.used(StallKind::Checks, "m2"), 1);
    }

    #[test]
    fn review_retries_grow_then_give_up() {
        let mut stalls = Stalls::default();
        assert_eq!(plan_review(&mut stalls, "m3"), Some((1, 60_000)));
        assert_eq!(plan_review(&mut stalls, "m3"), Some((2, 120_000)));
        assert_eq!(plan_review(&mut stalls, "m3"), Some((3, 240_000)));
        assert_eq!(plan_review(&mut stalls, "m3"), None);
        assert_eq!(review_delay(0), None);
        assert_eq!(review_delay(4), None);
    }

    #[test]
    fn recovers_only_with_room() {
        let base = RecoverInput { rounds: 2, max_rounds: 10, cap_allows: true };
        assert!(can_recover(&base));
        assert!(!can_recover(&RecoverInput { rounds: 10, ..base }));
        assert!(!can_recover(&RecoverInput { cap_allows: false, ..base }));
    }
}
