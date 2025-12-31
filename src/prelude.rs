//! Convenient re-exports for common types and traits

pub use crate::core::{
    BoxedJob, CancellationToken, ClosureJob, Job, JobHandle, Result, ThreadError,
};
#[cfg(feature = "priority-scheduling")]
pub use crate::core::{Priority, PriorityJob, PriorityQueue};
pub use crate::pool::{ThreadPool, ThreadPoolConfig, WorkerStats};
pub use crate::queue::{BoundedQueue, ChannelQueue, JobQueue, QueueCapabilities, QueueError, QueueResult};
#[cfg(feature = "priority-scheduling")]
pub use crate::queue::PriorityJobQueue;
