//! Queue factory for capability-based queue creation.
//!
//! This module provides a factory pattern for creating queue implementations
//! based on specified requirements, enabling users to select the right queue
//! without understanding implementation details.
//!
//! # Example
//!
//! ```rust
//! use rust_thread_system::queue::{QueueFactory, QueueRequirements};
//! use std::time::Duration;
//!
//! // Create a queue matching specific requirements
//! let queue = QueueFactory::create(
//!     QueueRequirements::new()
//!         .bounded(1000)
//! ).unwrap();
//!
//! // Use preset configurations
//! let web_queue = QueueFactory::web_server(5000, Duration::from_secs(30));
//! let realtime_queue = QueueFactory::realtime(1000);
//! ```

#[cfg(feature = "priority-scheduling")]
use super::PriorityJobQueue;
use super::{
    AdaptiveQueue, AdaptiveQueueConfig, BackpressureStrategy, BoundedQueue, ChannelQueue, JobQueue,
};
use crate::core::{Result, ThreadError};
use std::sync::Arc;
use std::time::Duration;

/// Requirements for queue creation.
///
/// Use the builder pattern to specify what capabilities the queue should have.
/// The factory will select an appropriate implementation based on these requirements.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::queue::QueueRequirements;
///
/// let requirements = QueueRequirements::new()
///     .bounded(1000)
///     .lock_free();
/// ```
#[derive(Clone, Debug, Default)]
pub struct QueueRequirements {
    /// Maximum capacity (None = unbounded)
    capacity: Option<usize>,

    /// Require lock-free implementation
    lock_free: bool,

    /// Require priority scheduling support
    priority_support: bool,

    /// Require exact size reporting
    exact_size: bool,

    /// Require MPMC (multi-producer multi-consumer)
    mpmc: bool,

    /// Prefer adaptive optimization
    adaptive: bool,

    /// Backpressure strategy for bounded queues
    backpressure: Option<BackpressureStrategy>,
}

impl QueueRequirements {
    /// Creates a new empty requirements builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the queue to be bounded with the specified capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of jobs the queue can hold
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::queue::QueueRequirements;
    ///
    /// let req = QueueRequirements::new().bounded(1000);
    /// ```
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn bounded(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Sets the queue to be unbounded.
    ///
    /// This is the default behavior.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn unbounded(mut self) -> Self {
        self.capacity = None;
        self
    }

    /// Requires the queue to use lock-free algorithms.
    ///
    /// Lock-free queues avoid lock contention and are optimal for
    /// high-contention scenarios.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn lock_free(mut self) -> Self {
        self.lock_free = true;
        self
    }

    /// Requires the queue to support priority scheduling.
    ///
    /// Jobs can be submitted with different priority levels, and
    /// higher priority jobs are processed first.
    ///
    /// # Note
    ///
    /// Requires the `priority-scheduling` feature to be enabled.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_priority(mut self) -> Self {
        self.priority_support = true;
        self
    }

    /// Requires the queue to report exact size.
    ///
    /// Some queue implementations only provide approximate sizes.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn exact_size(mut self) -> Self {
        self.exact_size = true;
        self
    }

    /// Requires the queue to support MPMC (multi-producer multi-consumer).
    ///
    /// All built-in queues support MPMC by default.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn mpmc(mut self) -> Self {
        self.mpmc = true;
        self
    }

    /// Enables adaptive optimization for the queue.
    ///
    /// Adaptive queues automatically switch between strategies based on
    /// runtime conditions to optimize performance.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn adaptive(mut self) -> Self {
        self.adaptive = true;
        self
    }

    /// Sets the backpressure strategy for bounded queues.
    ///
    /// This controls how the queue handles submissions when full.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn backpressure(mut self, strategy: BackpressureStrategy) -> Self {
        self.backpressure = Some(strategy);
        self
    }

    /// Returns whether a bounded queue is required.
    pub fn is_bounded(&self) -> bool {
        self.capacity.is_some()
    }

    /// Returns the required capacity, if any.
    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// Returns whether lock-free is required.
    pub fn requires_lock_free(&self) -> bool {
        self.lock_free
    }

    /// Returns whether priority support is required.
    pub fn requires_priority(&self) -> bool {
        self.priority_support
    }

    /// Returns whether exact size is required.
    pub fn requires_exact_size(&self) -> bool {
        self.exact_size
    }

    /// Returns whether MPMC is required.
    pub fn requires_mpmc(&self) -> bool {
        self.mpmc
    }

    /// Returns whether adaptive behavior is preferred.
    pub fn prefers_adaptive(&self) -> bool {
        self.adaptive
    }

    /// Returns the backpressure strategy, if specified.
    pub fn backpressure_strategy(&self) -> Option<&BackpressureStrategy> {
        self.backpressure.as_ref()
    }
}

/// Factory for creating queue implementations based on requirements.
///
/// The factory selects the most appropriate queue implementation based on
/// the specified requirements. If requirements cannot be satisfied, an error
/// is returned.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::queue::{QueueFactory, QueueRequirements, JobQueue};
///
/// // Create with requirements
/// let queue = QueueFactory::create(
///     QueueRequirements::new().bounded(100)
/// ).unwrap();
///
/// // Use convenience methods
/// let default_queue = QueueFactory::default_queue();
/// let fast_queue = QueueFactory::high_throughput();
/// ```
pub struct QueueFactory;

impl QueueFactory {
    /// Creates a queue matching the specified requirements.
    ///
    /// Returns an error if the requirements cannot be satisfied.
    ///
    /// # Selection Logic
    ///
    /// | Requirements | Selected Queue |
    /// |-------------|----------------|
    /// | Default | `ChannelQueue` (unbounded) |
    /// | `bounded(N)` | `BoundedQueue` |
    /// | `adaptive()` | `AdaptiveQueue` |
    /// | `with_priority()` | `PriorityJobQueue` |
    ///
    /// # Errors
    ///
    /// Returns [`ThreadError::InvalidConfig`] if:
    /// - `lock_free` + `priority` (not supported)
    /// - `adaptive` + `bounded` (adaptive manages its own sizing)
    /// - `priority` without `priority-scheduling` feature
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::queue::{QueueFactory, QueueRequirements};
    ///
    /// let queue = QueueFactory::create(
    ///     QueueRequirements::new().bounded(1000)
    /// ).unwrap();
    /// assert!(queue.capabilities().is_bounded);
    /// ```
    pub fn create(requirements: QueueRequirements) -> Result<Arc<dyn JobQueue>> {
        Self::validate_requirements(&requirements)?;

        #[cfg(feature = "priority-scheduling")]
        if requirements.priority_support {
            return Ok(Arc::new(PriorityJobQueue::new()));
        }

        if requirements.adaptive {
            return Ok(Arc::new(AdaptiveQueue::new(AdaptiveQueueConfig::default())));
        }

        if let Some(capacity) = requirements.capacity {
            return Ok(Arc::new(BoundedQueue::new(capacity)));
        }

        // Default: unbounded channel queue
        Ok(Arc::new(ChannelQueue::unbounded()))
    }

    /// Creates a default unbounded queue.
    ///
    /// This is the most common configuration, suitable for most use cases
    /// where memory is not a concern.
    #[must_use]
    pub fn default_queue() -> Arc<dyn JobQueue> {
        Arc::new(ChannelQueue::unbounded())
    }

    /// Creates a queue optimized for high throughput.
    ///
    /// Uses an adaptive queue that automatically switches between
    /// mutex-based and lock-free strategies based on contention.
    #[must_use]
    pub fn high_throughput() -> Arc<dyn JobQueue> {
        Arc::new(AdaptiveQueue::new(AdaptiveQueueConfig::default()))
    }

    /// Creates a queue optimized for low latency.
    ///
    /// Uses a bounded queue with immediate rejection to avoid
    /// any blocking when the queue is full.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum queue size
    #[must_use]
    pub fn low_latency(capacity: usize) -> Arc<dyn JobQueue> {
        Arc::new(BoundedQueue::new(capacity))
    }

    /// Creates a queue with automatic optimization.
    ///
    /// The queue monitors runtime conditions and adjusts its
    /// strategy accordingly.
    #[must_use]
    pub fn auto_optimized() -> Arc<dyn JobQueue> {
        Arc::new(AdaptiveQueue::new(AdaptiveQueueConfig::default()))
    }

    /// Preset: Web server request handling.
    ///
    /// Creates a bounded queue suitable for web servers where
    /// requests should be rejected gracefully under high load.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of pending requests
    /// * `_timeout` - Request timeout (reserved for future use with
    ///   backpressure-aware queue implementations)
    #[must_use]
    pub fn web_server(capacity: usize, _timeout: Duration) -> Arc<dyn JobQueue> {
        Arc::new(BoundedQueue::new(capacity))
    }

    /// Preset: Background job processing.
    ///
    /// Creates an unbounded queue for background tasks where
    /// jobs should never be dropped.
    ///
    /// If the `priority-scheduling` feature is enabled, returns a
    /// priority queue for processing important jobs first.
    #[must_use]
    pub fn background_jobs() -> Arc<dyn JobQueue> {
        #[cfg(feature = "priority-scheduling")]
        {
            Arc::new(PriorityJobQueue::new())
        }
        #[cfg(not(feature = "priority-scheduling"))]
        {
            Arc::new(ChannelQueue::unbounded())
        }
    }

    /// Preset: Real-time event processing.
    ///
    /// Creates a bounded queue for real-time systems where
    /// events must be processed within timing constraints.
    /// Uses a bounded queue to prevent memory growth.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum event buffer size
    #[must_use]
    pub fn realtime(capacity: usize) -> Arc<dyn JobQueue> {
        Arc::new(BoundedQueue::new(capacity))
    }

    /// Preset: Data pipeline with adaptive load.
    ///
    /// Creates an adaptive queue for data pipelines that
    /// experience varying load patterns.
    #[must_use]
    pub fn data_pipeline() -> Arc<dyn JobQueue> {
        Arc::new(AdaptiveQueue::new(AdaptiveQueueConfig::default()))
    }

    /// Validates that requirements are compatible.
    fn validate_requirements(req: &QueueRequirements) -> Result<()> {
        // Lock-free + priority not supported
        if req.lock_free && req.priority_support {
            return Err(ThreadError::invalid_config(
                "queue requirements",
                "lock-free priority queue is not supported",
            ));
        }

        // Adaptive + bounded not supported
        if req.adaptive && req.capacity.is_some() {
            return Err(ThreadError::invalid_config(
                "queue requirements",
                "adaptive queue cannot be bounded (it manages its own sizing)",
            ));
        }

        // Priority requires feature flag
        #[cfg(not(feature = "priority-scheduling"))]
        if req.priority_support {
            return Err(ThreadError::invalid_config(
                "queue requirements",
                "priority scheduling requires the 'priority-scheduling' feature",
            ));
        }

        Ok(())
    }

    /// Checks if created queue satisfies the given requirements.
    ///
    /// This can be used to verify that a queue meets specific capabilities.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::queue::{QueueFactory, QueueRequirements, ChannelQueue};
    /// use std::sync::Arc;
    ///
    /// let queue = Arc::new(ChannelQueue::unbounded());
    /// let requirements = QueueRequirements::new().unbounded();
    ///
    /// assert!(QueueFactory::satisfies(&*queue, &requirements));
    /// ```
    pub fn satisfies(queue: &dyn JobQueue, requirements: &QueueRequirements) -> bool {
        let caps = queue.capabilities();

        // Check bounded requirement
        if let Some(required_cap) = requirements.capacity {
            if !caps.is_bounded {
                return false;
            }
            if let Some(actual_cap) = caps.capacity {
                if actual_cap < required_cap {
                    return false;
                }
            }
        }

        // Check lock-free requirement
        if requirements.lock_free && !caps.is_lock_free {
            return false;
        }

        // Check priority requirement
        if requirements.priority_support && !caps.supports_priority {
            return false;
        }

        // Check exact size requirement
        if requirements.exact_size && !caps.exact_size {
            return false;
        }

        true
    }

    /// Returns a human-readable description of what queue would be created.
    ///
    /// Useful for logging and debugging configuration.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::queue::{QueueFactory, QueueRequirements};
    ///
    /// let requirements = QueueRequirements::new().bounded(1000);
    /// let description = QueueFactory::describe(&requirements);
    /// println!("Queue selection: {}", description);
    /// ```
    pub fn describe(requirements: &QueueRequirements) -> String {
        if requirements.priority_support {
            return "PriorityJobQueue (priority scheduling)".to_string();
        }

        if requirements.adaptive {
            return "AdaptiveQueue (auto-optimizing)".to_string();
        }

        if let Some(capacity) = requirements.capacity {
            return format!("BoundedQueue (capacity: {})", capacity);
        }

        "ChannelQueue (unbounded)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_requirements_default() {
        let req = QueueRequirements::new();
        assert!(!req.is_bounded());
        assert!(req.capacity().is_none());
        assert!(!req.requires_lock_free());
        assert!(!req.requires_priority());
        assert!(!req.requires_exact_size());
        assert!(!req.requires_mpmc());
        assert!(!req.prefers_adaptive());
    }

    #[test]
    fn test_queue_requirements_bounded() {
        let req = QueueRequirements::new().bounded(1000);
        assert!(req.is_bounded());
        assert_eq!(req.capacity(), Some(1000));
    }

    #[test]
    fn test_queue_requirements_unbounded() {
        let req = QueueRequirements::new().bounded(100).unbounded();
        assert!(!req.is_bounded());
        assert!(req.capacity().is_none());
    }

    #[test]
    fn test_queue_requirements_builder_chain() {
        let req = QueueRequirements::new()
            .bounded(500)
            .lock_free()
            .exact_size()
            .mpmc();

        assert!(req.is_bounded());
        assert_eq!(req.capacity(), Some(500));
        assert!(req.requires_lock_free());
        assert!(req.requires_exact_size());
        assert!(req.requires_mpmc());
    }

    #[test]
    fn test_factory_create_default() {
        let queue = QueueFactory::create(QueueRequirements::new()).unwrap();
        let caps = queue.capabilities();
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
    }

    #[test]
    fn test_factory_create_bounded() {
        let queue = QueueFactory::create(QueueRequirements::new().bounded(100)).unwrap();
        let caps = queue.capabilities();
        assert!(caps.is_bounded);
        assert_eq!(caps.capacity, Some(100));
    }

    #[test]
    fn test_factory_create_adaptive() {
        let queue = QueueFactory::create(QueueRequirements::new().adaptive()).unwrap();
        let caps = queue.capabilities();
        assert!(!caps.is_bounded); // Adaptive is unbounded
    }

    #[test]
    fn test_factory_invalid_lockfree_priority() {
        let result = QueueFactory::create(QueueRequirements::new().lock_free().with_priority());
        assert!(result.is_err());
    }

    #[test]
    fn test_factory_invalid_adaptive_bounded() {
        let result = QueueFactory::create(QueueRequirements::new().adaptive().bounded(100));
        assert!(result.is_err());
    }

    #[test]
    fn test_factory_default_queue() {
        let queue = QueueFactory::default_queue();
        let caps = queue.capabilities();
        assert!(!caps.is_bounded);
    }

    #[test]
    fn test_factory_high_throughput() {
        let queue = QueueFactory::high_throughput();
        let caps = queue.capabilities();
        assert!(!caps.is_bounded); // Adaptive is unbounded
    }

    #[test]
    fn test_factory_low_latency() {
        let queue = QueueFactory::low_latency(500);
        let caps = queue.capabilities();
        assert!(caps.is_bounded);
        assert_eq!(caps.capacity, Some(500));
    }

    #[test]
    fn test_factory_auto_optimized() {
        let queue = QueueFactory::auto_optimized();
        // Should be adaptive
        assert!(!queue.capabilities().is_bounded);
    }

    #[test]
    fn test_factory_web_server() {
        let queue = QueueFactory::web_server(5000, Duration::from_secs(30));
        let caps = queue.capabilities();
        assert!(caps.is_bounded);
        assert_eq!(caps.capacity, Some(5000));
    }

    #[test]
    fn test_factory_background_jobs() {
        let queue = QueueFactory::background_jobs();
        // Should be unbounded or priority-enabled
        let caps = queue.capabilities();
        #[cfg(feature = "priority-scheduling")]
        assert!(caps.supports_priority);
        #[cfg(not(feature = "priority-scheduling"))]
        assert!(!caps.is_bounded);
    }

    #[test]
    fn test_factory_realtime() {
        let queue = QueueFactory::realtime(1000);
        let caps = queue.capabilities();
        assert!(caps.is_bounded);
        assert_eq!(caps.capacity, Some(1000));
    }

    #[test]
    fn test_factory_data_pipeline() {
        let queue = QueueFactory::data_pipeline();
        // Should be adaptive
        assert!(!queue.capabilities().is_bounded);
    }

    #[test]
    fn test_factory_satisfies() {
        let queue = QueueFactory::create(QueueRequirements::new().bounded(100)).unwrap();

        // Should satisfy bounded requirement with smaller capacity
        assert!(QueueFactory::satisfies(
            &*queue,
            &QueueRequirements::new().bounded(50)
        ));

        // Should satisfy bounded requirement with exact capacity
        assert!(QueueFactory::satisfies(
            &*queue,
            &QueueRequirements::new().bounded(100)
        ));
    }

    #[test]
    fn test_factory_describe() {
        assert_eq!(
            QueueFactory::describe(&QueueRequirements::new()),
            "ChannelQueue (unbounded)"
        );
        assert_eq!(
            QueueFactory::describe(&QueueRequirements::new().bounded(100)),
            "BoundedQueue (capacity: 100)"
        );
        assert_eq!(
            QueueFactory::describe(&QueueRequirements::new().adaptive()),
            "AdaptiveQueue (auto-optimizing)"
        );
    }

    #[cfg(feature = "priority-scheduling")]
    #[test]
    fn test_factory_create_priority() {
        let queue = QueueFactory::create(QueueRequirements::new().with_priority()).unwrap();
        let caps = queue.capabilities();
        assert!(caps.supports_priority);
    }

    #[cfg(feature = "priority-scheduling")]
    #[test]
    fn test_factory_describe_priority() {
        assert_eq!(
            QueueFactory::describe(&QueueRequirements::new().with_priority()),
            "PriorityJobQueue (priority scheduling)"
        );
    }

    #[test]
    fn test_queue_requirements_backpressure() {
        let req = QueueRequirements::new()
            .bounded(100)
            .backpressure(BackpressureStrategy::RejectImmediately);

        assert!(req.backpressure_strategy().is_some());
        match req.backpressure_strategy() {
            Some(BackpressureStrategy::RejectImmediately) => {}
            _ => panic!("Expected RejectImmediately strategy"),
        }
    }

    #[test]
    fn test_queue_can_be_used_after_creation() {
        use crate::core::ClosureJob;

        let queue = QueueFactory::create(QueueRequirements::new()).unwrap();

        // Send a job
        let job = Box::new(ClosureJob::new(|| Ok(())));
        queue.send(job).unwrap();

        // Receive the job
        let received = queue.try_recv().unwrap();
        assert_eq!(received.job_type(), "ClosureJob");
    }
}
