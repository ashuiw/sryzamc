use std::{cmp::Ordering, collections::BinaryHeap, time::Duration};
use tokio::time::{interval, MissedTickBehavior};
use tracing::debug;
#[derive(Debug, Eq, PartialEq)]
struct Scheduled {
    at: u64,
    id: u64,
}
impl Ord for Scheduled {
    fn cmp(&self, o: &Self) -> Ordering {
        o.at.cmp(&self.at).then_with(|| o.id.cmp(&self.id))
    }
}
impl PartialOrd for Scheduled {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
/// Monotonic tick clock and due-task ordering; gameplay mutation remains tick-thread owned.
pub struct TickScheduler {
    tick: u64,
    tasks: BinaryHeap<Scheduled>,
    #[cfg(test)]
    next_id: u64,
    period: Duration,
}
impl TickScheduler {
    pub fn new(tps: u8) -> Self {
        Self {
            tick: 0,
            tasks: BinaryHeap::new(),
            #[cfg(test)]
            next_id: 0,
            period: Duration::from_nanos(1_000_000_000 / u64::from(tps.max(1))),
        }
    }
    #[cfg(test)]
    pub fn schedule_after(&mut self, delay: u64) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(Scheduled {
            at: self.tick.saturating_add(delay),
            id,
        });
        id
    }
    pub async fn run_clock(mut self) -> ! {
        let mut timer = interval(self.period);
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            self.tick += 1;
            while self.tasks.peek().is_some_and(|t| t.at <= self.tick) {
                let task = self.tasks.pop().unwrap();
                debug!(tick = self.tick, task = task.id, "task due");
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn due_order() {
        let mut s = TickScheduler::new(20);
        s.schedule_after(3);
        s.schedule_after(1);
        assert_eq!(s.tasks.peek().unwrap().at, 1);
    }
}
