use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};
use tokio::sync::watch;

#[derive(Default)]
pub(super) struct RunFeeds {
    senders: Mutex<BTreeMap<String, watch::Sender<u64>>>,
}

impl RunFeeds {
    pub(super) fn subscribe(&self, run_id: &str, sequence: u64) -> watch::Receiver<u64> {
        let mut senders = lock(&self.senders);
        senders
            .entry(run_id.into())
            .or_insert_with(|| watch::channel(sequence).0)
            .subscribe()
    }

    pub(super) fn publish(&self, run_id: &str, sequence: u64, terminal: bool) {
        let mut senders = lock(&self.senders);
        if let Some(sender) = senders.get(run_id) {
            sender.send_replace(sequence);
        } else if !terminal {
            senders.insert(run_id.into(), watch::channel(sequence).0);
        }
        if terminal {
            senders.remove(run_id);
        }
    }

    pub(super) fn close(&self, run_id: &str) {
        lock(&self.senders).remove(run_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.senders).len()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_terminal_replays_does_not_retain_feed_entries() {
        let feeds = RunFeeds::default();
        for index in 0..1_000 {
            let run_id = format!("terminal-{index}");
            let _receiver = feeds.subscribe(&run_id, 2);
            feeds.close(&run_id);
        }
        assert_eq!(feeds.len(), 0);
    }
}
