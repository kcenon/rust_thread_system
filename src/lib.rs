//! # Rust Thread System
//!
//! A Rust implementation of a thread system for efficient and safe concurrent programming.
//!
//! This library provides a reusable threading system that simplifies complex multithreading tasks.
//! It leverages Rust's ownership and type system to enable efficient and safe concurrent programming.
//!
//! ## Key Components
//!
//! * **Thread Base Module**: Foundational traits for thread operations
//! * **Logging System**: Thread-safe logging with multiple backends
//! * **Thread Pool System**: Efficient job distribution among worker threads
//! * **Priority Thread Pool**: Job processing based on priority levels
//!
//! ## Example
//!
//! ```
//! use rust_thread_system::{ThreadPool, job::CallbackJob};
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), rust_thread_system::error::Error> {
//! // Create a thread pool with 4 workers
//! let pool = ThreadPool::with_workers(4);
//!
//! // Start the thread pool
//! pool.start()?;
//!
//! // Create and submit a job
//! let job = CallbackJob::new(|| {
//!     println!("Hello from a thread pool job!");
//!     Ok(())
//! }, "hello_job");
//!
//! pool.submit(job)?;
//!
//! // Wait for job to complete
//! std::thread::sleep(Duration::from_millis(100));
//!
//! // Stop the thread pool
//! pool.stop(true);
//! # Ok(())
//! # }
//! ```

// Public modules
pub mod error;
pub mod job;
pub mod thread_base;
pub mod thread_pool;
pub mod priority_thread_pool;
pub mod pauseable_thread_pool;
pub mod logger;
pub mod utils;
pub mod job_pool_stats;
pub mod job_persistence;
pub mod backoff;
pub mod job_monitor;

// Re-exports of commonly used types
pub use thread_pool::ThreadPool;
pub use thread_base::ThreadWorker;
pub use priority_thread_pool::{PriorityThreadPool, JobPriority};
pub use pauseable_thread_pool::PauseableThreadPool;
pub use job_pool_stats::{JobPoolStats, JobPoolStatsSnapshot};
pub use job_persistence::{
    JobPersistenceManager, PersistableJob, JobDeserializer, 
    SerializableJob, PersistableCallbackJob, PersistableCallbackJobDeserializer
};
pub use backoff::{
    BackoffStrategy, RetryPolicy, ConstantBackoff, LinearBackoff, 
    ExponentialBackoff, CompositeBackoff
};
pub use job::RetryableJob;
pub use job_monitor::{JobMonitor, JobMonitorConfig, MonitoredJob, JobStats};

/// Returns the version of the library
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_exists() {
        assert!(!super::version().is_empty());
    }
}