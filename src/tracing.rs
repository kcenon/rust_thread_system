//! Tracing integration for observability.
//!
//! This module provides structured logging, metrics, and distributed tracing support
//! when the `tracing` feature is enabled.
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_thread_system::prelude::*;
//! use tracing_subscriber::{fmt, prelude::*, EnvFilter};
//!
//! // Set up tracing subscriber
//! tracing_subscriber::registry()
//!     .with(fmt::layer())
//!     .with(EnvFilter::from_default_env()
//!         .add_directive("rust_thread_system=debug".parse().unwrap()))
//!     .init();
//!
//! let pool = ThreadPool::with_threads(4)?;
//! pool.start()?;
//!
//! // Submit with tracing context propagation
//! pool.submit_traced(MyJob::new())?;
//! ```

use crate::core::{Job, Result};
use std::time::Duration;

/// A job wrapper that propagates tracing context across thread boundaries.
///
/// When a job is wrapped in `TracedJob`, the current tracing span is captured
/// at submission time and entered when the job executes, allowing distributed
/// tracing to work correctly across worker threads.
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::prelude::*;
///
/// let pool = ThreadPool::with_threads(4)?;
/// pool.start()?;
///
/// // Using submit_traced for automatic context propagation
/// pool.submit_traced(MyJob::new())?;
/// ```
pub struct TracedJob<J: Job> {
    inner: J,
    #[cfg(feature = "tracing")]
    span: tracing::Span,
}

impl<J: Job> TracedJob<J> {
    /// Creates a new TracedJob wrapping the given job.
    ///
    /// The current tracing span is captured and will be entered
    /// when the job executes.
    pub fn new(job: J) -> Self {
        Self {
            inner: job,
            #[cfg(feature = "tracing")]
            span: tracing::Span::current(),
        }
    }

    /// Creates a TracedJob with a specific span.
    #[cfg(feature = "tracing")]
    pub fn with_span(job: J, span: tracing::Span) -> Self {
        Self { inner: job, span }
    }
}

impl<J: Job> Job for TracedJob<J> {
    fn execute(&mut self) -> Result<()> {
        #[cfg(feature = "tracing")]
        let _guard = self.span.enter();
        self.inner.execute()
    }

    fn job_type(&self) -> &str {
        self.inner.job_type()
    }

    fn is_cancellable(&self) -> bool {
        self.inner.is_cancellable()
    }
}

/// Metrics recording functions for observability.
///
/// These functions emit tracing events that can be consumed by
/// metrics collection systems like Prometheus via tracing-opentelemetry.
#[cfg(feature = "tracing")]
pub mod metrics {
    use super::*;

    /// Records a job submission event.
    #[inline]
    pub fn record_submission(queue_depth: usize) {
        tracing::trace!(
            counter.jobs_submitted = 1,
            gauge.queue_depth = queue_depth as i64,
            "job submitted"
        );
    }

    /// Records job completion with timing.
    #[inline]
    pub fn record_completion(duration: Duration, success: bool) {
        let duration_ms = duration.as_millis() as u64;
        if success {
            tracing::trace!(
                counter.jobs_completed = 1,
                histogram.job_duration_ms = duration_ms,
                "job completed successfully"
            );
        } else {
            tracing::trace!(
                counter.jobs_failed = 1,
                histogram.job_duration_ms = duration_ms,
                "job failed"
            );
        }
    }

    /// Records a job panic event.
    #[inline]
    pub fn record_panic(duration: Duration) {
        tracing::trace!(
            counter.jobs_panicked = 1,
            histogram.job_duration_ms = duration.as_millis() as u64,
            "job panicked"
        );
    }

    /// Records worker becoming busy.
    #[inline]
    pub fn record_worker_busy(worker_id: usize) {
        tracing::trace!(
            gauge.workers_busy = 1,
            worker_id = worker_id,
            "worker busy"
        );
    }

    /// Records worker becoming idle.
    #[inline]
    pub fn record_worker_idle(worker_id: usize) {
        tracing::trace!(
            gauge.workers_busy = -1i64,
            worker_id = worker_id,
            "worker idle"
        );
    }

    /// Records pool startup.
    #[inline]
    pub fn record_pool_start(num_workers: usize, queue_type: &str) {
        tracing::info!(
            workers = num_workers,
            queue_type = queue_type,
            "thread pool started"
        );
    }

    /// Records pool shutdown.
    #[inline]
    pub fn record_pool_shutdown(jobs_processed: u64, jobs_failed: u64) {
        tracing::info!(
            jobs_processed = jobs_processed,
            jobs_failed = jobs_failed,
            "thread pool shutdown complete"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ClosureJob;

    #[test]
    fn test_traced_job_executes() {
        let executed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let executed_clone = executed.clone();

        let job = ClosureJob::new(move || {
            executed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });

        let mut traced = TracedJob::new(job);
        traced.execute().expect("Job should execute");

        assert!(executed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_traced_job_preserves_job_type() {
        let job = ClosureJob::new(|| Ok(()));
        let traced = TracedJob::new(job);

        assert_eq!(traced.job_type(), "ClosureJob");
    }
}
