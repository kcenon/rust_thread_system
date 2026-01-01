//! Tracing integration example
//!
//! This example demonstrates how to use the tracing feature for observability.
//!
//! Run with: `cargo run --example tracing_example --features tracing`
//!
//! Set RUST_LOG environment variable to control log levels:
//! - `RUST_LOG=trace` - Show all trace events including metrics
//! - `RUST_LOG=debug` - Show job execution details
//! - `RUST_LOG=info` - Show pool start/shutdown
//! - `RUST_LOG=rust_thread_system=debug` - Show only this crate's debug logs

use rust_thread_system::prelude::*;
use std::time::Duration;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// A sample job that simulates work
struct ComputeJob {
    id: u32,
    duration_ms: u64,
}

impl ComputeJob {
    fn new(id: u32, duration_ms: u64) -> Self {
        Self { id, duration_ms }
    }
}

impl Job for ComputeJob {
    fn execute(&mut self) -> Result<()> {
        tracing::info!(job_id = self.id, "starting computation");

        // Simulate work
        std::thread::sleep(Duration::from_millis(self.duration_ms));

        tracing::info!(job_id = self.id, "computation completed");
        Ok(())
    }

    fn job_type(&self) -> &str {
        "ComputeJob"
    }
}

/// A job that may fail
struct FailableJob {
    id: u32,
    should_fail: bool,
}

impl Job for FailableJob {
    fn execute(&mut self) -> Result<()> {
        if self.should_fail {
            Err(ThreadError::other(format!("Job {} failed intentionally", self.id)))
        } else {
            Ok(())
        }
    }

    fn job_type(&self) -> &str {
        "FailableJob"
    }
}

fn main() -> Result<()> {
    // Set up tracing subscriber with environment filter
    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true).with_thread_ids(true))
        .with(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Default to showing info and above for this crate, warn for others
                EnvFilter::new("warn,rust_thread_system=info")
            }),
        )
        .init();

    tracing::info!("starting tracing example");

    // Create and start a thread pool
    let pool = ThreadPool::with_threads(4)?;
    pool.start()?;

    // Submit regular jobs
    tracing::info!("submitting regular jobs");
    for i in 0..5 {
        pool.submit(ComputeJob::new(i, 50))?;
    }

    // Submit traced jobs with context propagation
    #[cfg(feature = "tracing")]
    {
        tracing::info!("submitting traced jobs with context propagation");

        // Create a parent span for a batch of related jobs
        let batch_span = tracing::info_span!("batch_processing", batch_id = 1);
        let _guard = batch_span.enter();

        for i in 5..8 {
            pool.submit_traced(ComputeJob::new(i, 30))?;
        }
    }

    // Submit some jobs that will fail
    tracing::info!("submitting failable jobs");
    for i in 0..3 {
        pool.submit(FailableJob {
            id: i,
            should_fail: i % 2 == 0,
        })?;
    }

    // Wait for jobs to complete
    std::thread::sleep(Duration::from_millis(500));

    // Log statistics
    tracing::info!(
        total_submitted = pool.total_jobs_submitted(),
        total_processed = pool.total_jobs_processed(),
        total_failed = pool.total_jobs_failed(),
        "job statistics"
    );

    // Shutdown the pool
    pool.shutdown()?;

    tracing::info!("tracing example completed");

    Ok(())
}
