//! In-memory (and persisted) utilization history used to forecast quota run-out from the
//! recent consumption rate instead of the whole-window average.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

/// Samples older than this are dropped.
pub const KEEP_SECS: i64 = 90 * 60;
/// Window used for the recent-rate estimate.
pub const RECENT_WINDOW_SECS: i64 = 45 * 60;
/// Minimum span between first and last recent sample for a usable rate.
pub const MIN_RATE_SPAN_SECS: i64 = 5 * 60;
/// No increase for this long (with enough history) means usage is idle.
pub const IDLE_SECS: i64 = 15 * 60;
/// Hard cap on samples per field.
pub const MAX_SAMPLES: usize = 360;
/// A utilization drop larger than this (percentage points) is treated as a window reset.
pub const RESET_DROP: f64 = 1.0;
/// resets_at values closer than this are the same window (APIs jitter by sub-seconds).
const RESETS_AT_TOLERANCE_SECS: i64 = 120;
/// Minimum seconds between disk writes.
pub const SAVE_INTERVAL_SECS: i64 = 60;
const FILE_NAME: &str = "usage-history.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FieldHistory {
    #[serde(default)]
    pub resets_at: String,
    #[serde(default)]
    pub samples: VecDeque<(i64, f64)>,
}

pub type History = HashMap<String, FieldHistory>;

/// Precomputed recent-consumption signal for one field.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecentRate {
    /// Utilization points per second over the recent window, when measurable.
    pub rate: Option<f64>,
    /// History is long enough and shows no increase for the idle period.
    pub idle: bool,
}

pub fn key(source_id: &str, field: &str) -> String {
    format!("{source_id}|{field}")
}

fn same_window(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (chrono::DateTime::parse_from_rfc3339(a), chrono::DateTime::parse_from_rfc3339(b)) {
        (Ok(x), Ok(y)) => (x - y).num_seconds().abs() <= RESETS_AT_TOLERANCE_SECS,
        _ => false,
    }
}

/// Appends a sample. Clears the field on a window reset (utilization drop or changed resets_at).
pub fn record(history: &mut History, key: &str, resets_at: &str, util: f64, now: i64) {
    if !util.is_finite() {
        return;
    }
    let entry = history.entry(key.to_string()).or_default();
    let dropped = entry.samples.back().map(|(_, v)| util < *v - RESET_DROP).unwrap_or(false);
    if dropped || (!entry.samples.is_empty() && !same_window(&entry.resets_at, resets_at)) {
        entry.samples.clear();
    }
    entry.resets_at = resets_at.to_string();
    // Ignore out-of-order timestamps.
    if entry.samples.back().map(|(t, _)| now < *t).unwrap_or(false) {
        return;
    }
    // Collapse runs of identical values to (first, latest) so the time of the last change survives.
    let n = entry.samples.len();
    if n >= 2 && entry.samples[n - 1].1 == util && entry.samples[n - 2].1 == util {
        entry.samples[n - 1].0 = now;
    } else {
        entry.samples.push_back((now, util));
    }
    prune_field(entry, now);
}

fn prune_field(entry: &mut FieldHistory, now: i64) {
    while entry.samples.front().map(|(t, _)| now - *t > KEEP_SECS).unwrap_or(false) {
        entry.samples.pop_front();
    }
    while entry.samples.len() > MAX_SAMPLES {
        entry.samples.pop_front();
    }
}

/// Drops stale samples and empty fields.
pub fn prune(history: &mut History, now: i64) {
    for entry in history.values_mut() {
        prune_field(entry, now);
    }
    history.retain(|_, e| !e.samples.is_empty());
}

/// Recent rate and idle state from raw samples (oldest first).
pub fn analyze(samples: &VecDeque<(i64, f64)>, now: i64) -> RecentRate {
    let Some(&(first_ts, _)) = samples.front() else { return RecentRate::default() };
    let Some(&(_, current)) = samples.back() else { return RecentRate::default() };
    let mut out = RecentRate::default();
    // Idle: history reaches back past the idle horizon and the value then equals the value now.
    if now - first_ts >= IDLE_SECS {
        let cutoff = now - IDLE_SECS;
        if let Some(&(_, then)) = samples.iter().rev().find(|(t, _)| *t <= cutoff) {
            out.idle = current <= then;
        }
    }
    let recent: Vec<&(i64, f64)> = samples.iter().filter(|(t, _)| now - *t <= RECENT_WINDOW_SECS).collect();
    if recent.len() >= 2 {
        let (t0, v0) = *recent[0];
        let (t1, v1) = *recent[recent.len() - 1];
        if t1 - t0 >= MIN_RATE_SPAN_SECS && v1 > v0 {
            out.rate = Some((v1 - v0) / (t1 - t0) as f64);
        }
    }
    out
}

/// Recent-rate signals for every field of one source.
pub fn rates_for(history: &History, source_id: &str, now: i64) -> HashMap<String, RecentRate> {
    let prefix = format!("{source_id}|");
    history
        .iter()
        .filter_map(|(k, e)| k.strip_prefix(&prefix).map(|f| (f.to_string(), analyze(&e.samples, now))))
        .collect()
}

pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn data_path() -> PathBuf {
    crate::settings::data_dir().join(FILE_NAME)
}

/// Loads history; a missing or corrupt file yields an empty map. Stale samples are discarded.
pub fn load_from(path: &Path, now: i64) -> History {
    let Ok(text) = std::fs::read_to_string(path) else { return History::new() };
    let mut history: History = serde_json::from_str(&text).unwrap_or_default();
    prune(&mut history, now);
    history
}

/// Atomic write (temp file + rename). Errors are returned for the caller to ignore.
pub fn save_to(path: &Path, history: &History) -> std::io::Result<()> {
    let text = serde_json::to_string(history).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    if std::fs::rename(&tmp, path).is_err() {
        // Windows may refuse to rename over an existing file in rare cases.
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path)?;
    }
    Ok(())
}

static LAST_SAVE: AtomicI64 = AtomicI64::new(0);

/// Saves at most once per `SAVE_INTERVAL_SECS` unless `force`.
pub fn save_throttled(history: &History, now: i64, force: bool) {
    if !force && now - LAST_SAVE.load(Ordering::Relaxed) < SAVE_INTERVAL_SECS {
        return;
    }
    LAST_SAVE.store(now, Ordering::Relaxed);
    let _ = save_to(&data_path(), history);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(h: &mut History, resets: &str, util: f64, now: i64) {
        record(h, "s|five_hour", resets, util, now);
    }

    fn samples(h: &History) -> &VecDeque<(i64, f64)> {
        &h["s|five_hour"].samples
    }

    #[test]
    fn dedup_keeps_first_and_latest_of_run() {
        let mut h = History::new();
        for i in 0..10 {
            rec(&mut h, "r", 20.0, i * 60);
        }
        assert_eq!(samples(&h).len(), 2);
        assert_eq!(samples(&h)[0].0, 0);
        assert_eq!(samples(&h)[1].0, 540);
    }

    #[test]
    fn pruning_by_age_and_count() {
        let mut h = History::new();
        for i in 0..200 {
            rec(&mut h, "r", i as f64 * 0.1, i * 60);
        }
        let now = 199 * 60;
        assert!(samples(&h).iter().all(|(t, _)| now - t <= KEEP_SECS));
        assert!(samples(&h).len() <= MAX_SAMPLES);
        let mut big = History::new();
        for i in 0..1000 {
            rec(&mut big, "r", i as f64 * 0.001, i);
        }
        assert!(samples(&big).len() <= MAX_SAMPLES);
        prune(&mut h, now + KEEP_SECS * 2);
        assert!(h.is_empty());
    }

    #[test]
    fn reset_clears_history() {
        let mut h = History::new();
        rec(&mut h, "2030-01-01T05:00:00+00:00", 40.0, 0);
        rec(&mut h, "2030-01-01T05:00:00+00:00", 45.0, 60);
        // Utilization drop.
        rec(&mut h, "2030-01-01T05:00:00+00:00", 3.0, 120);
        assert_eq!(samples(&h).len(), 1);
        rec(&mut h, "2030-01-01T05:00:00.400+00:00", 4.0, 180);
        assert_eq!(samples(&h).len(), 2, "sub-second jitter is the same window");
        // Window moved materially.
        rec(&mut h, "2030-01-01T10:00:00+00:00", 5.0, 240);
        assert_eq!(samples(&h).len(), 1);
    }

    #[test]
    fn recent_rate_and_idle() {
        let mut h = History::new();
        // 10 -> 20 over 20 minutes.
        for i in 0..=4 {
            rec(&mut h, "r", 10.0 + i as f64 * 2.5, i * 300);
        }
        let now = 1200;
        let a = analyze(samples(&h), now);
        assert!(!a.idle);
        assert!((a.rate.unwrap() - 10.0 / 1200.0).abs() < 1e-9);

        // Too little span -> no rate.
        let mut short = History::new();
        rec(&mut short, "r", 10.0, 0);
        rec(&mut short, "r", 12.0, 120);
        assert_eq!(analyze(samples(&short), 120), RecentRate::default());

        // Flat for 30 minutes after an increase -> idle.
        let mut idle = History::new();
        rec(&mut idle, "r", 10.0, 0);
        rec(&mut idle, "r", 20.0, 300);
        for i in 1..=6 {
            rec(&mut idle, "r", 20.0, 300 + i * 300);
        }
        let a = analyze(samples(&idle), 2100);
        assert!(a.idle);

        // History shorter than the idle horizon is never idle.
        let mut fresh = History::new();
        rec(&mut fresh, "r", 20.0, 0);
        rec(&mut fresh, "r", 20.0, 600);
        assert!(!analyze(samples(&fresh), 600).idle);
    }

    #[test]
    fn persistence_round_trip_and_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("usage-monitor-history-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("h.json");
        let mut h = History::new();
        rec(&mut h, "r", 10.0, 1000);
        rec(&mut h, "r", 12.0, 1300);
        save_to(&path, &h).unwrap();
        let loaded = load_from(&path, 1400);
        assert_eq!(loaded["s|five_hour"].samples, h["s|five_hour"].samples);
        assert_eq!(loaded["s|five_hour"].resets_at, "r");
        // Stale samples are discarded on load.
        assert!(load_from(&path, 1000 + KEEP_SECS + 1000).is_empty());
        // Corrupt and missing files never panic.
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load_from(&path, 1400).is_empty());
        assert!(load_from(&dir.join("missing.json"), 1400).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rates_for_filters_by_source() {
        let mut h = History::new();
        record(&mut h, &key("a", "f"), "r", 10.0, 0);
        record(&mut h, &key("b", "g"), "r", 10.0, 0);
        let r = rates_for(&h, "a", 0);
        assert!(r.contains_key("f") && !r.contains_key("g"));
    }
}
