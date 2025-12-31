//! # Rust Thread System
//!
//! A production-ready, high-performance Rust threading framework with worker pools and job queues.
//!
//! ## Features
//!
//! - **Thread Pool**: Efficient worker pool with configurable thread count
//! - **Job Queue**: Bounded and unbounded job queues using crossbeam channels
//! - **Worker Statistics**: Track job processing metrics per worker
//! - **Thread Safety**: Built on parking_lot and crossbeam for optimal performance
//! - **Graceful Shutdown**: Clean shutdown with worker thread joining
//! - **Type Safety**: Strongly-typed job system with trait-based design
//!
//! ## Quick Start
//!
//! ```rust
//! use rust_thread_system::prelude::*;
//!
//! # fn main() -> Result<()> {
//! // Create and start a thread pool
//! let pool = ThreadPool::with_threads(4)?;
//! pool.start()?;
//!
//! // Submit jobs
//! for i in 0..10 {
//!     pool.execute(move || {
//!         println!("Job {} executing", i);
//!         Ok(())
//!     })?;
//! }
//!
//! // Shutdown gracefully
//! pool.shutdown()?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Thread Pool Configuration
//!
//! ```rust
//! use rust_thread_system::prelude::*;
//!
//! # fn main() -> Result<()> {
//! let config = ThreadPoolConfig::new(8)
//!     .with_max_queue_size(1000)
//!     .with_thread_name_prefix("my-worker");
//!
//! let pool = ThreadPool::with_config(config)?;
//! pool.start()?;
//! # pool.shutdown()?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Custom Jobs
//!
//! ```rust
//! use rust_thread_system::prelude::*;
//!
//! struct MyJob {
//!     data: String,
//! }
//!
//! impl Job for MyJob {
//!     fn execute(&mut self) -> Result<()> {
//!         println!("Processing: {}", self.data);
//!         Ok(())
//!     }
//!
//!     fn job_type(&self) -> &str {
//!         "MyJob"
//!     }
//! }
//!
//! # fn main() -> Result<()> {
//! # let pool = ThreadPool::with_threads(2)?;
//! # pool.start()?;
//! pool.submit(MyJob {
//!     data: "test".to_string(),
//! })?;
//! # pool.shutdown()?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Worker Statistics
//!
//! ```rust
//! use rust_thread_system::prelude::*;
//!
//! # fn main() -> Result<()> {
//! # let pool = ThreadPool::with_threads(2)?;
//! # pool.start()?;
//! # for _ in 0..10 {
//! #     pool.execute(|| Ok(()))?;
//! # }
//! # std::thread::sleep(std::time::Duration::from_millis(100));
//! // Get statistics
//! let stats = pool.get_stats();
//! for (i, stat) in stats.iter().enumerate() {
//!     println!("Worker {}: {} jobs processed", i, stat.get_jobs_processed());
//! }
//!
//! println!("Total jobs: {}", pool.total_jobs_processed());
//! # pool.shutdown()?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod core;
pub mod pool;
pub mod prelude;

pub use core::{BoxedJob, CancellationToken, ClosureJob, Job, JobHandle, Result, ThreadError};
pub use pool::{ThreadPool, ThreadPoolConfig, WorkerStats};
