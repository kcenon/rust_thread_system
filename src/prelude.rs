//! Convenient re-exports for common types and traits

pub use crate::core::{
    BoxedJob, CancellationToken, ClosureJob, Job, JobHandle, Result, ThreadError,
};
#[cfg(feature = "priority-scheduling")]
pub use crate::core::{Priority, PriorityJob, PriorityQueue};
pub use crate::pool::{ThreadPool, ThreadPoolConfig, WorkerStats};
#[cfg(feature = "priority-scheduling")]
pub use crate::queue::PriorityJobQueue;
pub use crate::queue::{
    require_capabilities, BoundedQueue, CapabilityFlags, ChannelQueue, JobQueue,
    MissingCapabilitiesError, QueueCapabilities, QueueError, QueueResult,
};
pub use crate::typed::{
    DefaultJobType, JobType, TypeStats, TypedClosureJob, TypedJob, TypedPoolConfig, TypedThreadPool,
};
