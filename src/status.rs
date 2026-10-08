//! Per-source health, shared between pollers and the API.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;

pub const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SourceStatus {
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Status(Arc<RwLock<BTreeMap<String, SourceStatus>>>);

impl Status {
    pub fn success(&self, source: &str, at: DateTime<Utc>) {
        let mut map = self.0.write().unwrap();
        let entry = map.entry(source.to_string()).or_default();
        entry.last_success = Some(at);
        entry.last_error = None;
        entry.consecutive_failures = 0;
    }

    pub fn failure(&self, source: &str, error: &anyhow::Error) -> u32 {
        let mut map = self.0.write().unwrap();
        let entry = map.entry(source.to_string()).or_default();
        entry.last_error = Some(format!("{error:#}"));
        entry.consecutive_failures += 1;
        entry.consecutive_failures
    }

    pub fn snapshot(&self) -> BTreeMap<String, SourceStatus> {
        self.0.read().unwrap().clone()
    }
}

/// Delay before the next poll: `base` normally, doubling per consecutive failure, capped.
pub fn backoff_delay(base: Duration, failures: u32) -> Duration {
    if failures == 0 {
        return base;
    }
    base.saturating_mul(1u32 << failures.min(10)).min(MAX_BACKOFF).max(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn backoff_doubles_and_caps() {
        let base = Duration::from_secs(60);
        assert_eq!(backoff_delay(base, 0), base);
        assert_eq!(backoff_delay(base, 1), Duration::from_secs(120));
        assert_eq!(backoff_delay(base, 3), Duration::from_secs(480));
        assert_eq!(backoff_delay(base, 20), MAX_BACKOFF);
        let long = Duration::from_secs(1200);
        assert_eq!(backoff_delay(long, 1), long, "never shorter than the base interval");
    }

    #[test]
    fn success_resets_failures() {
        let status = Status::default();
        assert_eq!(status.failure("mrms", &anyhow::anyhow!("boom")), 1);
        assert_eq!(status.failure("mrms", &anyhow::anyhow!("boom")), 2);
        let at = Utc.with_ymd_and_hms(2026, 10, 8, 17, 0, 0).unwrap();
        status.success("mrms", at);
        let snap = status.snapshot();
        assert_eq!(
            snap["mrms"],
            SourceStatus { last_success: Some(at), last_error: None, consecutive_failures: 0 }
        );
    }
}
