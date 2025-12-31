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
//! # Queue Capability Introspection
//!
//! All queues implement capability introspection via [`QueueCapabilities`], allowing
//! runtime querying of queue characteristics:
//!
//! ```rust,ignore
//! use rust_thread_system::queue::{ChannelQueue, JobQueue, CapabilityFlags};
//!
//! let queue = ChannelQueue::unbounded();
//! let caps = queue.capabilities();
//!
//! println!("Queue: {}", caps.describe());
//! // Output: "crossbeam::channel::unbounded: [unbounded, lock-free, mpmc]"
//!
//! // Check if queue supports required capabilities
//! if queue.supports(CapabilityFlags::LOCK_FREE | CapabilityFlags::MPMC) {
//!     println!("Queue meets requirements!");
//! }
//! ```
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
use bitflags::bitflags;
use std::time::Duration;

bitflags! {
    /// Flags for specifying required queue capabilities.
    ///
    /// These flags can be combined to specify multiple requirements that a queue
    /// must satisfy. Use with [`JobQueue::supports()`] or [`require_capabilities()`]
    /// to check if a queue meets the requirements.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::queue::{CapabilityFlags, ChannelQueue, JobQueue};
    ///
    /// let queue = ChannelQueue::unbounded();
    ///
    /// // Check for multiple capabilities
    /// let required = CapabilityFlags::LOCK_FREE | CapabilityFlags::MPMC;
    /// if queue.supports(required) {
    ///     println!("Queue meets all requirements");
    /// }
    /// ```
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct CapabilityFlags: u32 {
        /// Require bounded queue (has maximum capacity)
        const BOUNDED = 1 << 0;
        /// Require unbounded queue (no maximum capacity)
        const UNBOUNDED = 1 << 1;
        /// Require lock-free operations
        const LOCK_FREE = 1 << 2;
        /// Require wait-free operations (stronger than lock-free)
        const WAIT_FREE = 1 << 3;
        /// Require priority scheduling support
        const PRIORITY = 1 << 4;
        /// Require exact size reporting
        const EXACT_SIZE = 1 << 5;
        /// Require MPMC (multi-producer multi-consumer) support
        const MPMC = 1 << 6;
        /// Require SPSC (single-producer single-consumer) optimization
        const SPSC = 1 << 7;
        /// Require blocking operations support
        const BLOCKING = 1 << 8;
        /// Require timeout operations support
        const TIMEOUT = 1 << 9;
        /// Require adaptive behavior
        const ADAPTIVE = 1 << 10;
    }
}

/// Capabilities of a queue implementation for runtime introspection.
///
/// This struct provides detailed information about a queue's characteristics,
/// enabling code to adapt behavior based on queue capabilities at runtime.
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::queue::{ChannelQueue, JobQueue};
///
/// let queue = ChannelQueue::unbounded();
/// let caps = queue.capabilities();
///
/// // Check specific capabilities
/// if caps.is_lock_free {
///     println!("Using lock-free queue for high-contention scenario");
/// }
///
/// // Get a human-readable description
/// println!("Queue info: {}", caps.describe());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueCapabilities {
    /// Whether the queue has a maximum capacity
    pub is_bounded: bool,
    /// Maximum capacity if bounded (None for unbounded)
    pub capacity: Option<usize>,
    /// Whether the queue uses lock-free algorithms
    pub is_lock_free: bool,
    /// Whether operations are wait-free (stronger guarantee than lock-free)
    pub is_wait_free: bool,
    /// Whether the queue supports priority ordering
    pub supports_priority: bool,
    /// Whether `len()` returns an exact count (vs approximate)
    pub exact_size: bool,
    /// Whether the queue supports MPMC (multi-producer multi-consumer)
    pub mpmc: bool,
    /// Whether the queue is optimized for SPSC (single-producer single-consumer)
    pub spsc: bool,
    /// Whether the queue supports blocking operations
    pub supports_blocking: bool,
    /// Whether the queue supports timeout operations
    pub supports_timeout: bool,
    /// Whether the queue uses adaptive strategies
    pub is_adaptive: bool,
    /// Queue implementation name for debugging/logging
    pub implementation_name: &'static str,
}

impl Default for QueueCapabilities {
    fn default() -> Self {
        Self {
            is_bounded: false,
            capacity: None,
            is_lock_free: true,
            is_wait_free: false,
            supports_priority: false,
            exact_size: false,
            mpmc: true,
            spsc: false,
            supports_blocking: true,
            supports_timeout: true,
            is_adaptive: false,
            implementation_name: "unknown",
        }
    }
}

impl QueueCapabilities {
    /// Creates capabilities for a standard unbounded channel queue.
    pub fn unbounded_channel() -> Self {
        Self {
            is_bounded: false,
            capacity: None,
            is_lock_free: true,
            is_wait_free: false,
            supports_priority: false,
            exact_size: false,
            mpmc: true,
            spsc: false,
            supports_blocking: true,
            supports_timeout: true,
            is_adaptive: false,
            implementation_name: "crossbeam::channel::unbounded",
        }
    }

    /// Creates capabilities for a bounded channel queue.
    pub fn bounded_channel(capacity: usize) -> Self {
        Self {
            is_bounded: true,
            capacity: Some(capacity),
            is_lock_free: true,
            is_wait_free: false,
            supports_priority: false,
            exact_size: false,
            mpmc: true,
            spsc: false,
            supports_blocking: true,
            supports_timeout: true,
            is_adaptive: false,
            implementation_name: "crossbeam::channel::bounded",
        }
    }

    /// Creates capabilities for a priority queue.
    pub fn priority_queue(capacity: Option<usize>) -> Self {
        Self {
            is_bounded: capacity.is_some(),
            capacity,
            is_lock_free: false,
            is_wait_free: false,
            supports_priority: true,
            exact_size: true,
            mpmc: true,
            spsc: false,
            supports_blocking: true,
            supports_timeout: true,
            is_adaptive: false,
            implementation_name: "PriorityJobQueue",
        }
    }

    /// Creates capabilities for an adaptive queue.
    pub fn adaptive() -> Self {
        Self {
            is_bounded: false,
            capacity: None,
            is_lock_free: false, // depends on current strategy
            is_wait_free: false,
            supports_priority: false,
            exact_size: false,
            mpmc: true,
            spsc: false,
            supports_blocking: true,
            supports_timeout: true,
            is_adaptive: true,
            implementation_name: "AdaptiveQueue",
        }
    }

    /// Returns a human-readable description of the queue capabilities.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::queue::{ChannelQueue, JobQueue};
    ///
    /// let queue = ChannelQueue::unbounded();
    /// println!("{}", queue.capabilities().describe());
    /// // Output: "crossbeam::channel::unbounded: [unbounded, lock-free, mpmc]"
    /// ```
    pub fn describe(&self) -> String {
        let mut features = Vec::new();

        if self.is_bounded {
            if let Some(cap) = self.capacity {
                features.push(format!("bounded({})", cap));
            } else {
                features.push("bounded".to_string());
            }
        } else {
            features.push("unbounded".to_string());
        }

        if self.is_wait_free {
            features.push("wait-free".to_string());
        } else if self.is_lock_free {
            features.push("lock-free".to_string());
        }

        if self.supports_priority {
            features.push("priority".to_string());
        }

        if self.is_adaptive {
            features.push("adaptive".to_string());
        }

        if self.mpmc {
            features.push("mpmc".to_string());
        } else if self.spsc {
            features.push("spsc".to_string());
        }

        if self.exact_size {
            features.push("exact-size".to_string());
        }

        format!("{}: [{}]", self.implementation_name, features.join(", "))
    }

    /// Checks if these capabilities satisfy the given flags.
    ///
    /// Returns `true` if all required capabilities are present.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::queue::{QueueCapabilities, CapabilityFlags};
    ///
    /// let caps = QueueCapabilities::unbounded_channel();
    ///
    /// // Check single requirement
    /// assert!(caps.satisfies(CapabilityFlags::LOCK_FREE));
    ///
    /// // Check multiple requirements
    /// assert!(caps.satisfies(CapabilityFlags::LOCK_FREE | CapabilityFlags::MPMC));
    ///
    /// // This will fail because unbounded channels are not bounded
    /// assert!(!caps.satisfies(CapabilityFlags::BOUNDED));
    /// ```
    pub fn satisfies(&self, flags: CapabilityFlags) -> bool {
        if flags.contains(CapabilityFlags::BOUNDED) && !self.is_bounded {
            return false;
        }
        if flags.contains(CapabilityFlags::UNBOUNDED) && self.is_bounded {
            return false;
        }
        if flags.contains(CapabilityFlags::LOCK_FREE) && !self.is_lock_free {
            return false;
        }
        if flags.contains(CapabilityFlags::WAIT_FREE) && !self.is_wait_free {
            return false;
        }
        if flags.contains(CapabilityFlags::PRIORITY) && !self.supports_priority {
            return false;
        }
        if flags.contains(CapabilityFlags::EXACT_SIZE) && !self.exact_size {
            return false;
        }
        if flags.contains(CapabilityFlags::MPMC) && !self.mpmc {
            return false;
        }
        if flags.contains(CapabilityFlags::SPSC) && !self.spsc {
            return false;
        }
        if flags.contains(CapabilityFlags::BLOCKING) && !self.supports_blocking {
            return false;
        }
        if flags.contains(CapabilityFlags::TIMEOUT) && !self.supports_timeout {
            return false;
        }
        if flags.contains(CapabilityFlags::ADAPTIVE) && !self.is_adaptive {
            return false;
        }
        true
    }
}

/// Checks if a queue supports the required capabilities.
///
/// Returns `Ok(())` if the queue satisfies all requirements, or an error
/// describing which capabilities are missing.
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::queue::{require_capabilities, ChannelQueue, JobQueue, CapabilityFlags};
///
/// let queue = ChannelQueue::unbounded();
///
/// // This will succeed
/// require_capabilities(&queue, CapabilityFlags::LOCK_FREE)?;
///
/// // This will fail with an error
/// require_capabilities(&queue, CapabilityFlags::PRIORITY)?;
/// ```
pub fn require_capabilities(
    queue: &dyn JobQueue,
    flags: CapabilityFlags,
) -> Result<(), MissingCapabilitiesError> {
    let caps = queue.capabilities();
    if caps.satisfies(flags) {
        Ok(())
    } else {
        Err(MissingCapabilitiesError {
            required: flags,
            actual: caps,
        })
    }
}

/// Error returned when a queue does not support required capabilities.
#[derive(Debug, Clone)]
pub struct MissingCapabilitiesError {
    /// The capabilities that were required
    pub required: CapabilityFlags,
    /// The actual capabilities of the queue
    pub actual: QueueCapabilities,
}

impl std::fmt::Display for MissingCapabilitiesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut missing = Vec::new();

        if self.required.contains(CapabilityFlags::BOUNDED) && !self.actual.is_bounded {
            missing.push("bounded");
        }
        if self.required.contains(CapabilityFlags::UNBOUNDED) && self.actual.is_bounded {
            missing.push("unbounded");
        }
        if self.required.contains(CapabilityFlags::LOCK_FREE) && !self.actual.is_lock_free {
            missing.push("lock-free");
        }
        if self.required.contains(CapabilityFlags::WAIT_FREE) && !self.actual.is_wait_free {
            missing.push("wait-free");
        }
        if self.required.contains(CapabilityFlags::PRIORITY) && !self.actual.supports_priority {
            missing.push("priority");
        }
        if self.required.contains(CapabilityFlags::EXACT_SIZE) && !self.actual.exact_size {
            missing.push("exact-size");
        }
        if self.required.contains(CapabilityFlags::MPMC) && !self.actual.mpmc {
            missing.push("mpmc");
        }
        if self.required.contains(CapabilityFlags::SPSC) && !self.actual.spsc {
            missing.push("spsc");
        }
        if self.required.contains(CapabilityFlags::BLOCKING) && !self.actual.supports_blocking {
            missing.push("blocking");
        }
        if self.required.contains(CapabilityFlags::TIMEOUT) && !self.actual.supports_timeout {
            missing.push("timeout");
        }
        if self.required.contains(CapabilityFlags::ADAPTIVE) && !self.actual.is_adaptive {
            missing.push("adaptive");
        }

        write!(
            f,
            "queue '{}' is missing required capabilities: [{}]",
            self.actual.implementation_name,
            missing.join(", ")
        )
    }
}

impl std::error::Error for MissingCapabilitiesError {}

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

    /// Checks if this queue supports the required capabilities.
    ///
    /// This is a convenience method that combines [`capabilities()`](Self::capabilities)
    /// with [`QueueCapabilities::satisfies()`].
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::queue::{ChannelQueue, JobQueue, CapabilityFlags};
    ///
    /// let queue = ChannelQueue::unbounded();
    ///
    /// if queue.supports(CapabilityFlags::LOCK_FREE | CapabilityFlags::MPMC) {
    ///     println!("Queue is suitable for high-contention MPMC scenario");
    /// }
    /// ```
    fn supports(&self, flags: CapabilityFlags) -> bool {
        self.capabilities().satisfies(flags)
    }

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
        assert!(!caps.is_wait_free);
        assert!(!caps.supports_priority);
        assert!(!caps.exact_size);
        assert!(caps.mpmc);
        assert!(!caps.spsc);
        assert!(caps.supports_blocking);
        assert!(caps.supports_timeout);
        assert!(!caps.is_adaptive);
        assert_eq!(caps.implementation_name, "unknown");
    }

    #[test]
    fn test_queue_capabilities_unbounded_channel() {
        let caps = QueueCapabilities::unbounded_channel();
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
        assert!(caps.is_lock_free);
        assert!(!caps.is_wait_free);
        assert!(!caps.supports_priority);
        assert!(!caps.exact_size);
        assert!(caps.mpmc);
        assert!(!caps.spsc);
        assert!(caps.supports_blocking);
        assert!(caps.supports_timeout);
        assert!(!caps.is_adaptive);
        assert_eq!(caps.implementation_name, "crossbeam::channel::unbounded");
    }

    #[test]
    fn test_queue_capabilities_bounded_channel() {
        let caps = QueueCapabilities::bounded_channel(100);
        assert!(caps.is_bounded);
        assert_eq!(caps.capacity, Some(100));
        assert!(caps.is_lock_free);
        assert!(!caps.is_wait_free);
        assert!(!caps.supports_priority);
        assert!(!caps.exact_size);
        assert!(caps.mpmc);
        assert!(!caps.spsc);
        assert!(caps.supports_blocking);
        assert!(caps.supports_timeout);
        assert!(!caps.is_adaptive);
        assert_eq!(caps.implementation_name, "crossbeam::channel::bounded");
    }

    #[test]
    fn test_queue_capabilities_priority_queue() {
        let caps = QueueCapabilities::priority_queue(None);
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
        assert!(!caps.is_lock_free);
        assert!(!caps.is_wait_free);
        assert!(caps.supports_priority);
        assert!(caps.exact_size);
        assert!(caps.mpmc);
        assert!(!caps.spsc);
        assert!(caps.supports_blocking);
        assert!(caps.supports_timeout);
        assert!(!caps.is_adaptive);
        assert_eq!(caps.implementation_name, "PriorityJobQueue");
    }

    #[test]
    fn test_queue_capabilities_adaptive() {
        let caps = QueueCapabilities::adaptive();
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
        assert!(!caps.is_lock_free);
        assert!(!caps.is_wait_free);
        assert!(!caps.supports_priority);
        assert!(!caps.exact_size);
        assert!(caps.mpmc);
        assert!(!caps.spsc);
        assert!(caps.supports_blocking);
        assert!(caps.supports_timeout);
        assert!(caps.is_adaptive);
        assert_eq!(caps.implementation_name, "AdaptiveQueue");
    }

    #[test]
    fn test_queue_capabilities_describe() {
        let caps = QueueCapabilities::unbounded_channel();
        let desc = caps.describe();
        assert!(desc.contains("crossbeam::channel::unbounded"));
        assert!(desc.contains("unbounded"));
        assert!(desc.contains("lock-free"));
        assert!(desc.contains("mpmc"));

        let caps = QueueCapabilities::bounded_channel(100);
        let desc = caps.describe();
        assert!(desc.contains("bounded(100)"));

        let caps = QueueCapabilities::priority_queue(None);
        let desc = caps.describe();
        assert!(desc.contains("priority"));
        assert!(desc.contains("exact-size"));

        let caps = QueueCapabilities::adaptive();
        let desc = caps.describe();
        assert!(desc.contains("adaptive"));
    }

    #[test]
    fn test_queue_capabilities_satisfies() {
        let caps = QueueCapabilities::unbounded_channel();

        // Should satisfy
        assert!(caps.satisfies(CapabilityFlags::LOCK_FREE));
        assert!(caps.satisfies(CapabilityFlags::UNBOUNDED));
        assert!(caps.satisfies(CapabilityFlags::MPMC));
        assert!(caps.satisfies(CapabilityFlags::BLOCKING));
        assert!(caps.satisfies(CapabilityFlags::TIMEOUT));
        assert!(caps.satisfies(CapabilityFlags::LOCK_FREE | CapabilityFlags::MPMC));

        // Should not satisfy
        assert!(!caps.satisfies(CapabilityFlags::BOUNDED));
        assert!(!caps.satisfies(CapabilityFlags::PRIORITY));
        assert!(!caps.satisfies(CapabilityFlags::WAIT_FREE));
        assert!(!caps.satisfies(CapabilityFlags::SPSC));
        assert!(!caps.satisfies(CapabilityFlags::ADAPTIVE));
        assert!(!caps.satisfies(CapabilityFlags::EXACT_SIZE));

        // Mixed flags - should fail if any required flag is missing
        assert!(!caps.satisfies(CapabilityFlags::LOCK_FREE | CapabilityFlags::PRIORITY));
    }

    #[test]
    fn test_capability_flags_combinations() {
        let flags = CapabilityFlags::LOCK_FREE | CapabilityFlags::MPMC;
        assert!(flags.contains(CapabilityFlags::LOCK_FREE));
        assert!(flags.contains(CapabilityFlags::MPMC));
        assert!(!flags.contains(CapabilityFlags::BOUNDED));

        let flags =
            CapabilityFlags::BOUNDED | CapabilityFlags::EXACT_SIZE | CapabilityFlags::PRIORITY;
        assert!(flags.contains(CapabilityFlags::BOUNDED));
        assert!(flags.contains(CapabilityFlags::EXACT_SIZE));
        assert!(flags.contains(CapabilityFlags::PRIORITY));
    }

    #[test]
    fn test_require_capabilities_success() {
        let queue = ChannelQueue::unbounded();
        assert!(require_capabilities(&queue, CapabilityFlags::LOCK_FREE).is_ok());
        assert!(require_capabilities(&queue, CapabilityFlags::UNBOUNDED).is_ok());
        assert!(require_capabilities(&queue, CapabilityFlags::MPMC).is_ok());
    }

    #[test]
    fn test_require_capabilities_failure() {
        let queue = ChannelQueue::unbounded();
        let result = require_capabilities(&queue, CapabilityFlags::PRIORITY);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("priority"));
        assert!(err.to_string().contains("crossbeam::channel::unbounded"));
    }

    #[test]
    fn test_missing_capabilities_error_display() {
        let caps = QueueCapabilities::unbounded_channel();
        let err = MissingCapabilitiesError {
            required: CapabilityFlags::PRIORITY | CapabilityFlags::BOUNDED,
            actual: caps,
        };
        let msg = err.to_string();
        assert!(msg.contains("priority"));
        assert!(msg.contains("bounded"));
        assert!(msg.contains("crossbeam::channel::unbounded"));
    }

    #[test]
    fn test_job_queue_supports() {
        let queue = ChannelQueue::unbounded();
        assert!(queue.supports(CapabilityFlags::LOCK_FREE));
        assert!(queue.supports(CapabilityFlags::MPMC));
        assert!(!queue.supports(CapabilityFlags::PRIORITY));
        assert!(!queue.supports(CapabilityFlags::BOUNDED));
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
