//! Per-type statistics for typed thread pool.
//!
//! This module provides [`TypeStats`] for tracking job metrics per job type.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Statistics for a specific job type.
///
/// Provides metrics for jobs of a particular type, including submission counts,
/// completion rates, and latency measurements.
#[derive(Clone, Debug, Default)]
pub struct TypeStats {
    /// Number of jobs submitted to this type's queue.
    pub jobs_submitted: u64,

    /// Number of jobs successfully completed.
    pub jobs_completed: u64,

    /// Number of jobs that failed with an error.
    pub jobs_failed: u64,

    /// Number of jobs that panicked during execution.
    pub jobs_panicked: u64,

    /// Current queue depth (approximate).
    pub queue_depth: usize,

    /// Average job execution latency.
    pub avg_latency: Duration,

    /// Maximum job execution latency observed.
    pub max_latency: Duration,

    /// Total execution time for all completed jobs.
    pub total_execution_time: Duration,
}

impl TypeStats {
    /// Returns the total number of jobs processed (completed + failed + panicked).
    pub fn jobs_processed(&self) -> u64 {
        self.jobs_completed + self.jobs_failed + self.jobs_panicked
    }

    /// Returns the success rate as a percentage (0.0 to 100.0).
    pub fn success_rate(&self) -> f64 {
        let processed = self.jobs_processed();
        if processed == 0 {
            100.0
        } else {
            (self.jobs_completed as f64 / processed as f64) * 100.0
        }
    }

    /// Returns the failure rate as a percentage (0.0 to 100.0).
    pub fn failure_rate(&self) -> f64 {
        100.0 - self.success_rate()
    }
}

/// Thread-safe statistics tracker for a job type.
///
/// Used internally by [`TypedThreadPool`] to track per-type metrics.
///
/// [`TypedThreadPool`]: crate::typed::TypedThreadPool
pub struct AtomicTypeStats {
    jobs_submitted: AtomicU64,
    jobs_completed: AtomicU64,
    jobs_failed: AtomicU64,
    jobs_panicked: AtomicU64,
    latency_tracker: Mutex<LatencyTracker>,
}

impl Default for AtomicTypeStats {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicTypeStats {
    /// Creates a new statistics tracker.
    pub fn new() -> Self {
        Self {
            jobs_submitted: AtomicU64::new(0),
            jobs_completed: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            jobs_panicked: AtomicU64::new(0),
            latency_tracker: Mutex::new(LatencyTracker::new()),
        }
    }

    /// Records a job submission.
    pub fn record_submission(&self) {
        self.jobs_submitted.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a successful job completion with execution duration.
    pub fn record_completion(&self, duration: Duration) {
        self.jobs_completed.fetch_add(1, Ordering::Relaxed);
        self.latency_tracker.lock().record(duration);
    }

    /// Records a job failure.
    pub fn record_failure(&self, duration: Duration) {
        self.jobs_failed.fetch_add(1, Ordering::Relaxed);
        self.latency_tracker.lock().record(duration);
    }

    /// Records a job panic.
    pub fn record_panic(&self, duration: Duration) {
        self.jobs_panicked.fetch_add(1, Ordering::Relaxed);
        self.latency_tracker.lock().record(duration);
    }

    /// Returns a snapshot of the current statistics.
    ///
    /// # Arguments
    ///
    /// * `queue_depth` - Current queue depth to include in the snapshot
    pub fn snapshot(&self, queue_depth: usize) -> TypeStats {
        let latency = self.latency_tracker.lock();
        TypeStats {
            jobs_submitted: self.jobs_submitted.load(Ordering::Relaxed),
            jobs_completed: self.jobs_completed.load(Ordering::Relaxed),
            jobs_failed: self.jobs_failed.load(Ordering::Relaxed),
            jobs_panicked: self.jobs_panicked.load(Ordering::Relaxed),
            queue_depth,
            avg_latency: latency.avg_latency(),
            max_latency: latency.max_latency,
            total_execution_time: latency.total_time,
        }
    }

    /// Returns the number of jobs submitted.
    pub fn jobs_submitted(&self) -> u64 {
        self.jobs_submitted.load(Ordering::Relaxed)
    }

    /// Returns the number of jobs completed.
    pub fn jobs_completed(&self) -> u64 {
        self.jobs_completed.load(Ordering::Relaxed)
    }

    /// Returns the number of jobs failed.
    pub fn jobs_failed(&self) -> u64 {
        self.jobs_failed.load(Ordering::Relaxed)
    }

    /// Returns the number of jobs panicked.
    pub fn jobs_panicked(&self) -> u64 {
        self.jobs_panicked.load(Ordering::Relaxed)
    }
}

/// Tracks latency metrics.
struct LatencyTracker {
    total_time: Duration,
    max_latency: Duration,
    count: u64,
}

impl LatencyTracker {
    fn new() -> Self {
        Self {
            total_time: Duration::ZERO,
            max_latency: Duration::ZERO,
            count: 0,
        }
    }

    fn record(&mut self, duration: Duration) {
        self.total_time += duration;
        self.max_latency = self.max_latency.max(duration);
        self.count += 1;
    }

    fn avg_latency(&self) -> Duration {
        if self.count == 0 {
            Duration::ZERO
        } else {
            self.total_time / self.count as u32
        }
    }
}

/// Helper for timing job execution.
pub struct ExecutionTimer {
    start: Instant,
}

impl ExecutionTimer {
    /// Starts a new execution timer.
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Returns the elapsed duration since the timer started.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_stats_default() {
        let stats = TypeStats::default();
        assert_eq!(stats.jobs_submitted, 0);
        assert_eq!(stats.jobs_completed, 0);
        assert_eq!(stats.jobs_failed, 0);
        assert_eq!(stats.jobs_panicked, 0);
    }

    #[test]
    fn test_type_stats_jobs_processed() {
        let stats = TypeStats {
            jobs_completed: 10,
            jobs_failed: 2,
            jobs_panicked: 1,
            ..Default::default()
        };
        assert_eq!(stats.jobs_processed(), 13);
    }

    #[test]
    fn test_type_stats_success_rate() {
        let stats = TypeStats {
            jobs_completed: 8,
            jobs_failed: 2,
            ..Default::default()
        };
        assert!((stats.success_rate() - 80.0).abs() < 0.01);
    }

    #[test]
    fn test_type_stats_success_rate_no_jobs() {
        let stats = TypeStats::default();
        assert!((stats.success_rate() - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_atomic_type_stats_submission() {
        let stats = AtomicTypeStats::new();
        stats.record_submission();
        stats.record_submission();
        assert_eq!(stats.jobs_submitted(), 2);
    }

    #[test]
    fn test_atomic_type_stats_completion() {
        let stats = AtomicTypeStats::new();
        stats.record_completion(Duration::from_millis(10));
        stats.record_completion(Duration::from_millis(20));
        assert_eq!(stats.jobs_completed(), 2);

        let snapshot = stats.snapshot(0);
        assert_eq!(snapshot.avg_latency, Duration::from_millis(15));
        assert_eq!(snapshot.max_latency, Duration::from_millis(20));
    }

    #[test]
    fn test_atomic_type_stats_failure() {
        let stats = AtomicTypeStats::new();
        stats.record_failure(Duration::from_millis(5));
        assert_eq!(stats.jobs_failed(), 1);
    }

    #[test]
    fn test_atomic_type_stats_panic() {
        let stats = AtomicTypeStats::new();
        stats.record_panic(Duration::from_millis(5));
        assert_eq!(stats.jobs_panicked(), 1);
    }

    #[test]
    fn test_atomic_type_stats_snapshot() {
        let stats = AtomicTypeStats::new();
        stats.record_submission();
        stats.record_completion(Duration::from_millis(10));

        let snapshot = stats.snapshot(5);
        assert_eq!(snapshot.jobs_submitted, 1);
        assert_eq!(snapshot.jobs_completed, 1);
        assert_eq!(snapshot.queue_depth, 5);
    }

    #[test]
    fn test_execution_timer() {
        let timer = ExecutionTimer::start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.elapsed();
        assert!(elapsed >= Duration::from_millis(10));
    }
}
