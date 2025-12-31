//! Backpressure strategies for bounded queues.
//!
//! This module provides configurable strategies for handling queue-full scenarios
//! in bounded queues, enabling graceful degradation under load.
//!
//! # Strategies
//!
//! - [`BackpressureStrategy::Block`]: Block until space is available (default)
//! - [`BackpressureStrategy::BlockWithTimeout`]: Block with timeout, return error if exceeded
//! - [`BackpressureStrategy::RejectImmediately`]: Return error immediately if queue is full
//! - [`BackpressureStrategy::DropOldest`]: Drop the oldest job to make room
//! - [`BackpressureStrategy::DropNewest`]: Drop the new job (keep existing queue intact)
//! - [`BackpressureStrategy::Custom`]: Use a custom handler
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_thread_system::prelude::*;
//! use rust_thread_system::queue::BackpressureStrategy;
//! use std::time::Duration;
//!
//! // Reject immediately for real-time systems
//! let config = ThreadPoolConfig::default()
//!     .with_max_queue_size(1000)
//!     .with_backpressure_strategy(BackpressureStrategy::RejectImmediately);
//!
//! // Timeout for web servers
//! let config = ThreadPoolConfig::default()
//!     .with_max_queue_size(5000)
//!     .with_backpressure_strategy(BackpressureStrategy::BlockWithTimeout(Duration::from_secs(5)));
//! ```

use crate::core::{BoxedJob, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Strategy for handling queue-full scenarios in bounded queues.
#[derive(Clone)]
pub enum BackpressureStrategy {
    /// Block until space is available (default behavior).
    Block,

    /// Block with timeout, return error if exceeded.
    BlockWithTimeout(Duration),

    /// Return error immediately if queue is full.
    RejectImmediately,

    /// Drop oldest job to make room for new one.
    ///
    /// Note: This strategy requires a special queue implementation that
    /// supports removing items from the front. If the underlying queue
    /// doesn't support this, it falls back to `RejectImmediately`.
    DropOldest,

    /// Drop the new job (keep existing queue intact).
    DropNewest,

    /// Call user-provided handler.
    Custom(Arc<dyn BackpressureHandler>),
}

impl std::fmt::Debug for BackpressureStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block => write!(f, "Block"),
            Self::BlockWithTimeout(d) => write!(f, "BlockWithTimeout({:?})", d),
            Self::RejectImmediately => write!(f, "RejectImmediately"),
            Self::DropOldest => write!(f, "DropOldest"),
            Self::DropNewest => write!(f, "DropNewest"),
            Self::Custom(_) => write!(f, "Custom(<handler>)"),
        }
    }
}

impl Default for BackpressureStrategy {
    fn default() -> Self {
        Self::Block
    }
}

/// Handler for custom backpressure logic.
///
/// Implement this trait to provide custom behavior when a bounded queue is full.
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::queue::BackpressureHandler;
/// use rust_thread_system::core::{BoxedJob, Result, ThreadError};
///
/// struct LogAndReject;
///
/// impl BackpressureHandler for LogAndReject {
///     fn handle_backpressure(&self, job: BoxedJob) -> Result<Option<BoxedJob>> {
///         eprintln!("Queue full, rejecting job of type: {}", job.job_type());
///         Err(ThreadError::queue_full(0, 0))
///     }
/// }
/// ```
pub trait BackpressureHandler: Send + Sync {
    /// Called when the queue is full.
    ///
    /// # Arguments
    ///
    /// * `job` - The job that could not be enqueued
    ///
    /// # Returns
    ///
    /// - `Ok(Some(job))` - Retry submission with the (possibly modified) job
    /// - `Ok(None)` - Drop the job silently
    /// - `Err(_)` - Propagate error to caller
    fn handle_backpressure(&self, job: BoxedJob) -> Result<Option<BoxedJob>>;
}

/// Statistics for backpressure events.
///
/// Tracks metrics related to backpressure handling.
#[derive(Debug, Default)]
pub struct BackpressureStats {
    /// Total jobs submitted
    jobs_submitted: AtomicU64,
    /// Jobs rejected due to RejectImmediately strategy
    jobs_rejected: AtomicU64,
    /// Jobs dropped due to DropOldest strategy
    jobs_dropped_oldest: AtomicU64,
    /// Jobs dropped due to DropNewest strategy
    jobs_dropped_newest: AtomicU64,
    /// Total backpressure events (any strategy triggered)
    backpressure_events: AtomicU64,
    /// Total timeout events
    timeout_events: AtomicU64,
}

impl BackpressureStats {
    /// Creates a new stats instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a successful job submission.
    pub fn record_submission(&self) {
        self.jobs_submitted.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a job rejection.
    pub fn record_rejection(&self) {
        self.jobs_rejected.fetch_add(1, Ordering::Relaxed);
        self.backpressure_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a dropped oldest job.
    pub fn record_dropped_oldest(&self) {
        self.jobs_dropped_oldest.fetch_add(1, Ordering::Relaxed);
        self.backpressure_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a dropped newest job.
    pub fn record_dropped_newest(&self) {
        self.jobs_dropped_newest.fetch_add(1, Ordering::Relaxed);
        self.backpressure_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a timeout event.
    pub fn record_timeout(&self) {
        self.timeout_events.fetch_add(1, Ordering::Relaxed);
        self.backpressure_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a backpressure event (for custom handlers).
    pub fn record_backpressure(&self) {
        self.backpressure_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Gets the total number of jobs submitted.
    pub fn jobs_submitted(&self) -> u64 {
        self.jobs_submitted.load(Ordering::Relaxed)
    }

    /// Gets the number of jobs rejected.
    pub fn jobs_rejected(&self) -> u64 {
        self.jobs_rejected.load(Ordering::Relaxed)
    }

    /// Gets the number of jobs dropped due to DropOldest.
    pub fn jobs_dropped_oldest(&self) -> u64 {
        self.jobs_dropped_oldest.load(Ordering::Relaxed)
    }

    /// Gets the number of jobs dropped due to DropNewest.
    pub fn jobs_dropped_newest(&self) -> u64 {
        self.jobs_dropped_newest.load(Ordering::Relaxed)
    }

    /// Gets the total number of backpressure events.
    pub fn backpressure_events(&self) -> u64 {
        self.backpressure_events.load(Ordering::Relaxed)
    }

    /// Gets the number of timeout events.
    pub fn timeout_events(&self) -> u64 {
        self.timeout_events.load(Ordering::Relaxed)
    }

    /// Returns a snapshot of all stats.
    pub fn snapshot(&self) -> BackpressureStatsSnapshot {
        BackpressureStatsSnapshot {
            jobs_submitted: self.jobs_submitted(),
            jobs_rejected: self.jobs_rejected(),
            jobs_dropped_oldest: self.jobs_dropped_oldest(),
            jobs_dropped_newest: self.jobs_dropped_newest(),
            backpressure_events: self.backpressure_events(),
            timeout_events: self.timeout_events(),
        }
    }
}

/// A snapshot of backpressure statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackpressureStatsSnapshot {
    /// Total jobs submitted
    pub jobs_submitted: u64,
    /// Jobs rejected due to RejectImmediately strategy
    pub jobs_rejected: u64,
    /// Jobs dropped due to DropOldest strategy
    pub jobs_dropped_oldest: u64,
    /// Jobs dropped due to DropNewest strategy
    pub jobs_dropped_newest: u64,
    /// Total backpressure events
    pub backpressure_events: u64,
    /// Total timeout events
    pub timeout_events: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backpressure_strategy_debug() {
        assert_eq!(format!("{:?}", BackpressureStrategy::Block), "Block");
        assert_eq!(
            format!(
                "{:?}",
                BackpressureStrategy::BlockWithTimeout(Duration::from_secs(1))
            ),
            "BlockWithTimeout(1s)"
        );
        assert_eq!(
            format!("{:?}", BackpressureStrategy::RejectImmediately),
            "RejectImmediately"
        );
        assert_eq!(format!("{:?}", BackpressureStrategy::DropOldest), "DropOldest");
        assert_eq!(format!("{:?}", BackpressureStrategy::DropNewest), "DropNewest");
    }

    #[test]
    fn test_backpressure_strategy_default() {
        let strategy = BackpressureStrategy::default();
        assert!(matches!(strategy, BackpressureStrategy::Block));
    }

    #[test]
    fn test_backpressure_stats() {
        let stats = BackpressureStats::new();

        stats.record_submission();
        stats.record_submission();
        assert_eq!(stats.jobs_submitted(), 2);

        stats.record_rejection();
        assert_eq!(stats.jobs_rejected(), 1);
        assert_eq!(stats.backpressure_events(), 1);

        stats.record_dropped_oldest();
        assert_eq!(stats.jobs_dropped_oldest(), 1);
        assert_eq!(stats.backpressure_events(), 2);

        stats.record_dropped_newest();
        assert_eq!(stats.jobs_dropped_newest(), 1);
        assert_eq!(stats.backpressure_events(), 3);

        stats.record_timeout();
        assert_eq!(stats.timeout_events(), 1);
        assert_eq!(stats.backpressure_events(), 4);
    }

    #[test]
    fn test_backpressure_stats_snapshot() {
        let stats = BackpressureStats::new();
        stats.record_submission();
        stats.record_rejection();

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.jobs_submitted, 1);
        assert_eq!(snapshot.jobs_rejected, 1);
    }

    struct TestHandler;

    impl BackpressureHandler for TestHandler {
        fn handle_backpressure(&self, _job: BoxedJob) -> Result<Option<BoxedJob>> {
            Ok(None)
        }
    }

    #[test]
    fn test_custom_strategy() {
        let handler = Arc::new(TestHandler);
        let strategy = BackpressureStrategy::Custom(handler);
        assert!(matches!(strategy, BackpressureStrategy::Custom(_)));
    }
}
