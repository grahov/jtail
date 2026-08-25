//! Wall-clock skew detection between remote journals and the local host.

/// Estimates the offset between a remote journal's timestamps and the local
/// clock from the first live entries of a stream.
///
/// Each sample is local arrival time minus entry timestamp. Transport and
/// scheduling delay only push the median up by fractions of a second, so a
/// median beyond a couple of seconds in either direction means the clocks
/// disagree and cross-host ordering cannot be trusted.
pub struct SkewTracker {
    samples: Vec<i64>,
    warned: bool,
}

const SAMPLE_TARGET: usize = 32;
const THRESHOLD_US: i64 = 2_000_000;

impl SkewTracker {
    pub fn new() -> Self {
        Self {
            samples: Vec::with_capacity(SAMPLE_TARGET),
            warned: false,
        }
    }

    /// Feeds one live entry; returns the estimated skew in microseconds the
    /// first time enough samples exist and their median crosses the
    /// reporting threshold.
    pub fn observe(&mut self, entry_ts_us: i64, arrival_us: i64) -> Option<i64> {
        if self.warned || self.samples.len() >= SAMPLE_TARGET {
            return None;
        }
        self.samples.push(arrival_us - entry_ts_us);
        if self.samples.len() < SAMPLE_TARGET {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        if median.abs() > THRESHOLD_US {
            self.warned = true;
            Some(median)
        } else {
            None
        }
    }
}

impl Default for SkewTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(tracker: &mut SkewTracker, offset_us: i64) -> Option<i64> {
        let mut result = None;
        for i in 0..SAMPLE_TARGET as i64 {
            result = tracker.observe(1_000_000 * i, 1_000_000 * i + offset_us);
        }
        result
    }

    #[test]
    fn reports_a_clearly_skewed_clock_once() {
        let mut t = SkewTracker::new();
        assert_eq!(feed(&mut t, 5_000_000), Some(5_000_000));
        assert_eq!(t.observe(0, 5_000_000), None);
    }

    #[test]
    fn tolerates_normal_delivery_delay() {
        let mut t = SkewTracker::new();
        assert_eq!(feed(&mut t, 150_000), None);
    }

    #[test]
    fn reports_a_remote_clock_running_ahead() {
        let mut t = SkewTracker::new();
        assert_eq!(feed(&mut t, -3_000_000), Some(-3_000_000));
    }
}
