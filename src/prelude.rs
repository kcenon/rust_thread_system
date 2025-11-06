//! Convenient re-exports for common types and traits

pub use crate::core::{
    BoxedJob, CancellationToken, ClosureJob, Job, JobHandle, Result, ThreadError,
};
pub use crate::pool::{
    BackpressureStrategy, JobResult, PoolStats, ThreadPool, ThreadPoolBuilder, ThreadPoolConfig,
    WorkerStats, WorkerStatSnapshot,
};
