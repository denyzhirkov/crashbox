//! Per-project fan-out for live logs: a bounded RAM ring buffer (scrollback) plus a `broadcast`
//! channel (live tail). The broadcast is **intentionally lossy** — a subscriber that falls behind
//! is dropped by tokio rather than growing an unbounded queue. This is a live tail, not a journal.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use tokio::sync::broadcast;

use crate::config::LiveLogConfig;

use super::LogRecord;

/// A connected stream: the scrollback snapshot taken at subscribe time, plus the live receiver.
pub struct Subscription {
    pub snapshot: Vec<Arc<LogRecord>>,
    pub rx: broadcast::Receiver<Arc<LogRecord>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SubscribeError {
    /// Concurrent subscribers for this project reached `max_subscribers_per_project`.
    TooManySubscribers,
}

struct ProjectChannel {
    tx: broadcast::Sender<Arc<LogRecord>>,
    ring: Mutex<VecDeque<Arc<LogRecord>>>,
}

pub struct LiveLogHub {
    enabled: bool,
    buffer_cap: usize,
    channel_cap: usize,
    max_subscribers: usize,
    projects: RwLock<HashMap<i64, Arc<ProjectChannel>>>,
}

impl LiveLogHub {
    pub fn from_config(cfg: &LiveLogConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            buffer_cap: cfg.buffer_per_project,
            // broadcast capacity must be >= 1; size it to the ring so a subscriber can lag by a
            // full scrollback window before being dropped.
            channel_cap: cfg.buffer_per_project.max(1),
            max_subscribers: cfg.max_subscribers_per_project,
            projects: RwLock::new(HashMap::new()),
        }
    }

    /// Append a record to the project ring and broadcast it to live subscribers. A send error
    /// (no subscribers) is expected and ignored.
    pub fn publish(&self, project_id: i64, record: LogRecord) {
        if !self.enabled {
            return;
        }
        let channel = self.channel(project_id);
        let record = Arc::new(record);
        {
            let mut ring = channel.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if self.buffer_cap == 0 {
                ring.clear();
            } else {
                while ring.len() >= self.buffer_cap {
                    ring.pop_front();
                }
                ring.push_back(record.clone());
            }
        }
        let _ = channel.tx.send(record);
    }

    /// Subscribe to a project's live stream. The receiver is registered **before** the snapshot is
    /// taken so no record is lost in the gap; a record may therefore appear in both — callers that
    /// care can dedupe by `Arc` identity.
    pub fn subscribe(&self, project_id: i64) -> Result<Subscription, SubscribeError> {
        let channel = self.channel(project_id);
        if channel.tx.receiver_count() >= self.max_subscribers {
            return Err(SubscribeError::TooManySubscribers);
        }
        let rx = channel.tx.subscribe();
        let snapshot = channel
            .ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        Ok(Subscription { snapshot, rx })
    }

    fn channel(&self, project_id: i64) -> Arc<ProjectChannel> {
        if let Some(c) = self
            .projects
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&project_id)
        {
            return c.clone();
        }
        let mut map = self
            .projects
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        map.entry(project_id)
            .or_insert_with(|| {
                Arc::new(ProjectChannel {
                    tx: broadcast::channel(self.channel_cap).0,
                    ring: Mutex::new(VecDeque::new()),
                })
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::livelog::LogLevel;
    use serde_json::json;

    fn cfg(buffer: usize, max_subs: usize) -> LiveLogConfig {
        LiveLogConfig {
            enabled: true,
            buffer_per_project: buffer,
            max_batch_bytes: 1024,
            message_max_bytes: 1024,
            max_per_minute_per_project: 1000,
            max_subscribers_per_project: max_subs,
        }
    }

    fn rec(msg: &str) -> LogRecord {
        LogRecord::from_loose(&json!({ "message": msg }), 1024).expect("rec")
    }

    #[test]
    fn ring_is_bounded_and_keeps_newest() {
        let hub = LiveLogHub::from_config(&cfg(3, 10));
        for i in 0..5 {
            hub.publish(1, rec(&format!("m{i}")));
        }
        let sub = hub.subscribe(1).expect("subscribe");
        let msgs: Vec<_> = sub.snapshot.iter().map(|r| r.message.clone()).collect();
        assert_eq!(msgs, vec!["m2", "m3", "m4"]);
    }

    #[tokio::test]
    async fn subscriber_receives_live_records() {
        let hub = LiveLogHub::from_config(&cfg(10, 10));
        let mut sub = hub.subscribe(7).expect("subscribe");
        assert!(sub.snapshot.is_empty());
        hub.publish(7, rec("live"));
        let got = sub.rx.recv().await.expect("recv");
        assert_eq!(got.message, "live");
        assert_eq!(got.level, LogLevel::Info);
    }

    #[test]
    fn projects_are_isolated() {
        let hub = LiveLogHub::from_config(&cfg(10, 10));
        hub.publish(1, rec("for-one"));
        assert_eq!(hub.subscribe(2).expect("subscribe").snapshot.len(), 0);
        assert_eq!(hub.subscribe(1).expect("subscribe").snapshot.len(), 1);
    }

    #[test]
    fn enforces_max_subscribers() {
        let hub = LiveLogHub::from_config(&cfg(10, 2));
        let _a = hub.subscribe(1).expect("first");
        let _b = hub.subscribe(1).expect("second");
        assert!(matches!(
            hub.subscribe(1),
            Err(SubscribeError::TooManySubscribers)
        ));
    }

    #[test]
    fn disabled_hub_publishes_nothing() {
        let mut c = cfg(10, 10);
        c.enabled = false;
        let hub = LiveLogHub::from_config(&c);
        hub.publish(1, rec("ignored"));
        assert!(hub.subscribe(1).expect("subscribe").snapshot.is_empty());
    }
}
