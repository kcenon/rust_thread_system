//! Typed thread pool for type-based job routing.
//!
//! This module provides a typed thread pool that routes jobs to dedicated
//! queues based on job type, enabling QoS guarantees and type-specific scheduling.
//!
//! # Overview
//!
//! The typed thread pool is designed for applications that need:
//! - **Per-type dedicated queues** preventing cross-type interference
//! - **QoS guarantees** for different job categories
//! - **Priority scheduling** across job types
//! - **Isolation** of slow jobs from fast paths
//!
//! # Components
//!
//! - [`JobType`]: Trait for defining job type categories
//! - [`DefaultJobType`]: Built-in job types (IO, Compute, Critical, Background)
//! - [`TypedJob`]: Trait for jobs that declare their type
//! - [`TypedClosureJob`]: Wrapper for typed closures
//! - [`TypedPoolConfig`]: Configuration builder for the pool
//! - [`TypedThreadPool`]: The main typed pool implementation
//! - [`TypeStats`]: Per-type statistics
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use rust_thread_system::typed::{TypedThreadPool, TypedPoolConfig, DefaultJobType};
//!
//! // Create a pool with custom worker counts per type
//! let config = TypedPoolConfig::<DefaultJobType>::new()
//!     .workers_for(DefaultJobType::Critical, 4)
//!     .workers_for(DefaultJobType::Compute, 8)
//!     .workers_for(DefaultJobType::Io, 16)
//!     .workers_for(DefaultJobType::Background, 2)
//!     .type_priority(vec![
//!         DefaultJobType::Critical,
//!         DefaultJobType::Io,
//!         DefaultJobType::Compute,
//!         DefaultJobType::Background,
//!     ]);
//!
//! let pool = TypedThreadPool::new(config)?;
//! pool.start()?;
//!
//! // Submit jobs with explicit types
//! pool.execute_typed(DefaultJobType::Critical, || {
//!     println!("High priority task!");
//!     Ok(())
//! })?;
//!
//! pool.execute_typed(DefaultJobType::Background, || {
//!     println!("Low priority cleanup");
//!     Ok(())
//! })?;
//!
//! pool.shutdown()?;
//! ```
//!
//! # Custom Job Types
//!
//! You can define your own job types for domain-specific categorization:
//!
//! ```rust
//! use rust_thread_system::typed::JobType;
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
//! enum GameJobType {
//!     Physics,
//!     Rendering,
//!     Audio,
//!     Network,
//!     AI,
//! }
//!
//! impl JobType for GameJobType {
//!     fn all_variants() -> &'static [Self] {
//!         &[Self::Physics, Self::Rendering, Self::Audio, Self::Network, Self::AI]
//!     }
//!
//!     fn default_type() -> Self {
//!         Self::Physics
//!     }
//! }
//! ```
//!
//! # Presets
//!
//! For common workload patterns, use the preset configurations:
//!
//! ```rust,ignore
//! use rust_thread_system::typed::{TypedPoolConfig, DefaultJobType};
//!
//! // For IO-heavy workloads (databases, network services)
//! let config = TypedPoolConfig::<DefaultJobType>::io_optimized();
//!
//! // For CPU-heavy workloads (data processing, computation)
//! let config = TypedPoolConfig::<DefaultJobType>::compute_optimized();
//!
//! // For mixed workloads
//! let config = TypedPoolConfig::<DefaultJobType>::balanced();
//! ```
//!
//! # Statistics
//!
//! The pool tracks per-type metrics for monitoring:
//!
//! ```rust,ignore
//! let stats = pool.type_stats(DefaultJobType::Io).unwrap();
//! println!("IO jobs: submitted={}, completed={}, failed={}",
//!     stats.jobs_submitted,
//!     stats.jobs_completed,
//!     stats.jobs_failed);
//! println!("Average latency: {:?}", stats.avg_latency);
//! ```

mod config;
mod job_type;
mod pool;
mod stats;
mod typed_job;
mod worker;

pub use config::TypedPoolConfig;
pub use job_type::{DefaultJobType, JobType};
pub use pool::TypedThreadPool;
pub use stats::{AtomicTypeStats, ExecutionTimer, TypeStats};
pub use typed_job::{TypedClosureJob, TypedJob};
pub use worker::TypedWorker;
