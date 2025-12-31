//! Queue abstractions for pluggable job queue implementations.
//!
//! This module provides the [`JobQueue`] trait that abstracts job queue behavior,
//! enabling multiple queue implementations to be used interchangeably with [`ThreadPool`].
//!
//! # Built-in Implementations
//!
//! - [`ChannelQueue`]: Unbounded FIFO queue using crossbeam channels (default)
//! - [`BoundedQueue`]: Bounded FIFO queue with configurable capacity
//! - [`AdaptiveQueue`]: Auto-optimizing queue that switches between mutex and lock-free
//! - [`PriorityJobQueue`]: Priority-based queue (requires `priority-scheduling` feature)
//!
//! # Custom Queues
//!
//! You can implement custom queues by implementing the [`JobQueue`] trait:
//!
//! ```rust,ignore
//! use rust_thread_system::queue::{JobQueue, QueueCapabilities, BoxedJob};
//!
//! struct MyCustomQueue { /* ... */ }
//!
//! impl JobQueue for MyCustomQueue {
//!     // Implement all required methods...
//! }
//! ```
//!
//! [`ThreadPool`]: crate::pool::ThreadPool

mod adaptive;
mod backpressure;
mod bounded;
mod channel;
mod factory;
#[cfg(feature = "priority-scheduling")]
mod priority;

pub use adaptive::{AdaptiveQueue, AdaptiveQueueConfig, AdaptiveQueueStats, QueueStrategy};
pub use backpressure::{
    BackpressureHandler, BackpressureStats, BackpressureStatsSnapshot, BackpressureStrategy,
};
pub use bounded::BoundedQueue;
pub use channel::ChannelQueue;
pub use factory::{QueueFactory, QueueRequirements};
#[cfg(feature = "priority-scheduling")]
pub use priority::PriorityJobQueue;

use crate::core::BoxedJob;
#[cfg(feature = "priority-scheduling")]
use crate::core::Priority;
use std::time::Duration;

/// Capabilities of a queue implementation for runtime introspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueCapabilities {
    /// Whether the queue has a maximum capacity
    pub is_bounded: bool,
    /// Maximum capacity if bounded (None for unbounded)
    pub capacity: Option<usize>,
    /// Whether the queue uses lock-free algorithms
    pub is_lock_free: bool,
    /// Whether the queue supports priority ordering
    pub supports_priority: bool,
    /// Whether `len()` returns an exact count (vs approximate)
    pub exact_size: bool,
}

impl Default for QueueCapabilities {
    fn default() -> Self {
        Self {
            is_bounded: false,
            capacity: None,
            is_lock_free: true,
            supports_priority: false,
            exact_size: false,
        }
    }
}

/// Errors that can occur during queue operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    /// Queue is full (for bounded queues)
    Full(BoxedJobHolder),
    /// Queue is closed and not accepting new jobs
    Closed(BoxedJobHolder),
    /// Queue is empty (for try_recv)
    Empty,
    /// Queue is disconnected (sender/receiver dropped)
    Disconnected,
    /// Operation timed out
    Timeout(BoxedJobHolder),
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::Full(_) => write!(f, "queue is full"),
            QueueError::Closed(_) => write!(f, "queue is closed"),
            QueueError::Empty => write!(f, "queue is empty"),
            QueueError::Disconnected => write!(f, "queue is disconnected"),
            QueueError::Timeout(_) => write!(f, "operation timed out"),
        }
    }
}

impl std::error::Error for QueueError {}

/// A holder for boxed jobs in error cases to allow recovery.
///
/// This wrapper enables returning the job back to the caller when
/// queue operations fail, allowing them to retry or handle the job differently.
#[derive(Debug)]
pub struct BoxedJobHolder {
    job: Option<BoxedJob>,
}

impl BoxedJobHolder {
    /// Creates a new holder with the given job.
    pub fn new(job: BoxedJob) -> Self {
        Self { job: Some(job) }
    }

    /// Takes the job out of the holder.
    pub fn take(mut self) -> Option<BoxedJob> {
        self.job.take()
    }

    /// Returns a reference to the job if present.
    pub fn as_ref(&self) -> Option<&BoxedJob> {
        self.job.as_ref()
    }
}

impl Clone for BoxedJobHolder {
    fn clone(&self) -> Self {
        // Jobs cannot be cloned, so we create an empty holder
        Self { job: None }
    }
}

impl PartialEq for BoxedJobHolder {
    fn eq(&self, other: &Self) -> bool {
        // Compare by presence of job only
        self.job.is_some() == other.job.is_some()
    }
}

impl Eq for BoxedJobHolder {}

/// Result type for queue operations.
pub type QueueResult<T> = std::result::Result<T, QueueError>;

/// Trait for job queue implementations.
///
/// This trait abstracts job queue behavior, enabling multiple queue implementations
/// to be used interchangeably with `ThreadPool`.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to allow sharing across threads.
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::queue::{JobQueue, ChannelQueue};
/// use std::sync::Arc;
///
/// let queue: Arc<dyn JobQueue> = Arc::new(ChannelQueue::unbounded());
/// ```
pub trait JobQueue: Send + Sync {
    /// Sends a job to the queue, blocking if necessary.
    ///
    /// For bounded queues, this will block until space is available.
    /// For unbounded queues, this should never block.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Closed`] if the queue has been closed.
    fn send(&self, job: BoxedJob) -> QueueResult<()>;

    /// Attempts to send a job without blocking.
    ///
    /// # Errors
    ///
    /// - [`QueueError::Full`] if the queue is full (bounded queues)
    /// - [`QueueError::Closed`] if the queue has been closed
    fn try_send(&self, job: BoxedJob) -> QueueResult<()>;

    /// Sends a job with a timeout.
    ///
    /// # Errors
    ///
    /// - [`QueueError::Timeout`] if the operation times out
    /// - [`QueueError::Closed`] if the queue has been closed
    fn send_timeout(&self, job: BoxedJob, timeout: Duration) -> QueueResult<()>;

    /// Receives a job from the queue, blocking until one is available.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Disconnected`] if the queue has been closed
    /// and is empty.
    fn recv(&self) -> QueueResult<BoxedJob>;

    /// Attempts to receive a job without blocking.
    ///
    /// # Returns
    ///
    /// - `Ok(job)` if a job was available
    /// - `Err(QueueError::Empty)` if no job was available
    /// - `Err(QueueError::Disconnected)` if the queue is closed and empty
    fn try_recv(&self) -> QueueResult<BoxedJob>;

    /// Receives a job with a timeout.
    ///
    /// # Returns
    ///
    /// - `Ok(job)` if a job was received within the timeout
    /// - `Err(QueueError::Empty)` if no job was available within the timeout
    /// - `Err(QueueError::Disconnected)` if the queue is closed and empty
    fn recv_timeout(&self, timeout: Duration) -> QueueResult<BoxedJob>;

    /// Closes the queue, preventing new jobs from being sent.
    ///
    /// Jobs already in the queue can still be received.
    fn close(&self);

    /// Returns `true` if the queue has been closed.
    fn is_closed(&self) -> bool;

    /// Returns the current number of items in the queue.
    ///
    /// Note: For some implementations, this may be an approximation.
    /// Check [`QueueCapabilities::exact_size`] to determine if this is exact.
    fn len(&self) -> usize;

    /// Returns `true` if the queue is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the capabilities of this queue implementation.
    fn capabilities(&self) -> QueueCapabilities;

    /// Sends a job with a specific priority.
    ///
    /// For queues that support priority scheduling, this will enqueue the job
    /// with the specified priority level. For queues that don't support priority,
    /// this falls back to regular [`send()`](Self::send), ignoring the priority.
    ///
    /// # Errors
    ///
    /// - [`QueueError::Closed`] if the queue has been closed.
    /// - [`QueueError::Full`] if the queue is bounded and at capacity.
    #[cfg(feature = "priority-scheduling")]
    fn send_with_priority(&self, job: BoxedJob, _priority: Priority) -> QueueResult<()> {
        // Default implementation ignores priority
        self.send(job)
    }

    /// Attempts to send a job with priority without blocking.
    ///
    /// For queues that support priority scheduling, this will try to enqueue the job
    /// with the specified priority level. For queues that don't support priority,
    /// this falls back to regular [`try_send()`](Self::try_send), ignoring the priority.
    ///
    /// # Errors
    ///
    /// - [`QueueError::Full`] if the queue is bounded and at capacity.
    /// - [`QueueError::Closed`] if the queue has been closed.
    #[cfg(feature = "priority-scheduling")]
    fn try_send_with_priority(&self, job: BoxedJob, _priority: Priority) -> QueueResult<()> {
        // Default implementation ignores priority
        self.try_send(job)
    }

    /// Sends a job with priority, with a timeout.
    ///
    /// For queues that support priority scheduling, this will try to enqueue the job
    /// with the specified priority level within the timeout. For queues that don't support
    /// priority, this falls back to regular [`send_timeout()`](Self::send_timeout),
    /// ignoring the priority.
    ///
    /// # Errors
    ///
    /// - [`QueueError::Timeout`] if the operation times out.
    /// - [`QueueError::Closed`] if the queue has been closed.
    #[cfg(feature = "priority-scheduling")]
    fn send_with_priority_timeout(
        &self,
        job: BoxedJob,
        _priority: Priority,
        timeout: Duration,
    ) -> QueueResult<()> {
        // Default implementation ignores priority
        self.send_timeout(job, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_capabilities_default() {
        let caps = QueueCapabilities::default();
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
        assert!(caps.is_lock_free);
        assert!(!caps.supports_priority);
        assert!(!caps.exact_size);
    }

    #[test]
    fn test_queue_error_display() {
        let holder = BoxedJobHolder { job: None };
        assert_eq!(
            QueueError::Full(holder.clone()).to_string(),
            "queue is full"
        );
        assert_eq!(
            QueueError::Closed(holder.clone()).to_string(),
            "queue is closed"
        );
        assert_eq!(QueueError::Empty.to_string(), "queue is empty");
        assert_eq!(
            QueueError::Disconnected.to_string(),
            "queue is disconnected"
        );
        assert_eq!(
            QueueError::Timeout(holder).to_string(),
            "operation timed out"
        );
    }

    #[test]
    fn test_boxed_job_holder() {
        let holder = BoxedJobHolder { job: None };
        assert!(holder.as_ref().is_none());
        assert!(holder.take().is_none());
    }
}
