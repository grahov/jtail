//! Cross-host ordering buffer.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::record::Record;

/// Buffers entries briefly so that streams from different hosts interleave
/// in timestamp order.
///
/// An entry is released once it has been buffered for the window, and only
/// after every earlier-stamped entry already buffered. Ordering across
/// hosts is therefore best-effort: an entry delayed by more than the window
/// is printed late, in arrival order relative to what already left the
/// buffer.
pub struct Merger {
    heap: BinaryHeap<Held>,
    window: Duration,
    seq: u64,
}

struct Held {
    timestamp_us: i64,
    seq: u64,
    arrived: Instant,
    record: Record,
}

impl Merger {
    pub fn new(window: Duration) -> Self {
        Self {
            heap: BinaryHeap::new(),
            window,
            seq: 0,
        }
    }

    pub fn push(&mut self, record: Record, now: Instant) {
        self.seq += 1;
        self.heap.push(Held {
            timestamp_us: record.timestamp_us,
            seq: self.seq,
            arrived: now,
            record,
        });
    }

    /// Releases, oldest timestamp first, every entry whose buffering window
    /// has elapsed.
    pub fn pop_ready(&mut self, now: Instant) -> Vec<Record> {
        let mut out = Vec::new();
        while let Some(held) = self.heap.peek() {
            if now.saturating_duration_since(held.arrived) < self.window {
                break;
            }
            out.push(self.heap.pop().expect("peeked entry exists").record);
        }
        out
    }

    /// Releases everything immediately, oldest timestamp first.
    pub fn drain(&mut self) -> Vec<Record> {
        let mut out = Vec::with_capacity(self.heap.len());
        while let Some(held) = self.heap.pop() {
            out.push(held.record);
        }
        out
    }
}

/* BinaryHeap is a max-heap; Held compares reversed so the heap yields the
smallest (timestamp, seq) first. */

impl Ord for Held {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.timestamp_us, other.seq).cmp(&(self.timestamp_us, self.seq))
    }
}

impl PartialOrd for Held {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp_us == other.timestamp_us && self.seq == other.seq
    }
}

impl Eq for Held {}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(host: &str, timestamp_us: i64) -> Record {
        Record {
            host: host.into(),
            timestamp_us,
            cursor: None,
            unit: None,
            pid: None,
            priority: None,
            message: String::new(),
        }
    }

    #[test]
    fn releases_in_timestamp_order() {
        let mut m = Merger::new(Duration::from_millis(300));
        let arrived = Instant::now();
        m.push(record("b", 200), arrived);
        m.push(record("a", 100), arrived);
        m.push(record("c", 300), arrived);
        let out = m.pop_ready(arrived + Duration::from_millis(301));
        let ts: Vec<i64> = out.iter().map(|r| r.timestamp_us).collect();
        assert_eq!(ts, vec![100, 200, 300]);
    }

    #[test]
    fn holds_entries_inside_the_window() {
        let mut m = Merger::new(Duration::from_millis(300));
        let arrived = Instant::now();
        m.push(record("a", 100), arrived);
        assert!(m.pop_ready(arrived + Duration::from_millis(100)).is_empty());
        assert_eq!(m.pop_ready(arrived + Duration::from_millis(300)).len(), 1);
    }

    #[test]
    fn preserves_arrival_order_for_equal_timestamps() {
        let mut m = Merger::new(Duration::ZERO);
        let arrived = Instant::now();
        m.push(record("first", 100), arrived);
        m.push(record("second", 100), arrived);
        let out = m.pop_ready(arrived);
        assert_eq!(out[0].host, "first");
        assert_eq!(out[1].host, "second");
    }

    #[test]
    fn drain_flushes_everything_in_order() {
        let mut m = Merger::new(Duration::from_secs(60));
        let arrived = Instant::now();
        m.push(record("b", 2), arrived);
        m.push(record("a", 1), arrived);
        let ts: Vec<i64> = m.drain().iter().map(|r| r.timestamp_us).collect();
        assert_eq!(ts, vec![1, 2]);
    }
}
