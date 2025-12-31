//! Configuration for typed thread pool.
//!
//! This module provides [`TypedPoolConfig`] for configuring per-type
//! worker counts, queue settings, and scheduling priorities.

use super::JobType;
use std::collections::HashMap;
use std::time::Duration;

/// Configuration for a typed thread pool.
///
/// Allows configuring per-type worker counts, queue settings, and
/// scheduling priorities.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::typed::{TypedPoolConfig, DefaultJobType};
/// use std::time::Duration;
///
/// let config = TypedPoolConfig::<DefaultJobType>::new()
///     .workers_for(DefaultJobType::Critical, 4)
///     .workers_for(DefaultJobType::Compute, 8)
///     .workers_for(DefaultJobType::Io, 16)
///     .workers_for(DefaultJobType::Background, 2)
///     .type_priority(vec![
///         DefaultJobType::Critical,
///         DefaultJobType::Io,
///         DefaultJobType::Compute,
///         DefaultJobType::Background,
///     ])
///     .with_poll_interval(Duration::from_millis(50));
/// ```
#[derive(Clone, Debug)]
pub struct TypedPoolConfig<T: JobType> {
    /// Workers per job type.
    pub workers_per_type: HashMap<T, usize>,

    /// Default worker count for types not explicitly configured.
    pub default_workers: usize,

    /// Queue capacity per type (0 = unbounded).
    pub queue_capacity_per_type: HashMap<T, usize>,

    /// Default queue capacity for types not explicitly configured.
    pub default_queue_capacity: usize,

    /// Worker poll interval for checking jobs and shutdown.
    pub poll_interval: Duration,

    /// Type priority ordering (first = highest priority).
    /// Workers will check queues in this order.
    pub type_priority: Vec<T>,

    /// Thread name prefix.
    pub thread_name_prefix: String,
}

impl<T: JobType> Default for TypedPoolConfig<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: JobType> TypedPoolConfig<T> {
    /// Creates a new configuration with sensible defaults.
    ///
    /// Default values:
    /// - 2 workers per type
    /// - Unbounded queues (capacity = 0)
    /// - 100ms poll interval
    /// - Natural priority ordering from `JobType::all_variants()`
    #[must_use]
    pub fn new() -> Self {
        Self {
            workers_per_type: HashMap::new(),
            default_workers: 2,
            queue_capacity_per_type: HashMap::new(),
            default_queue_capacity: 0,
            poll_interval: Duration::from_millis(100),
            type_priority: T::all_variants().to_vec(),
            thread_name_prefix: "typed-worker".to_string(),
        }
    }

    /// Sets the worker count for a specific job type.
    ///
    /// # Arguments
    ///
    /// * `job_type` - The job type to configure
    /// * `count` - Number of workers dedicated to this type
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn workers_for(mut self, job_type: T, count: usize) -> Self {
        self.workers_per_type.insert(job_type, count);
        self
    }

    /// Sets the default worker count for unconfigured types.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_default_workers(mut self, count: usize) -> Self {
        self.default_workers = count;
        self
    }

    /// Sets the queue capacity for a specific job type.
    ///
    /// # Arguments
    ///
    /// * `job_type` - The job type to configure
    /// * `capacity` - Queue capacity (0 = unbounded)
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn queue_capacity_for(mut self, job_type: T, capacity: usize) -> Self {
        self.queue_capacity_per_type.insert(job_type, capacity);
        self
    }

    /// Sets the default queue capacity for unconfigured types.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_default_queue_capacity(mut self, capacity: usize) -> Self {
        self.default_queue_capacity = capacity;
        self
    }

    /// Sets the type priority order.
    ///
    /// Workers check queues in this order, so types earlier in the list
    /// are processed first when multiple queues have pending jobs.
    ///
    /// # Arguments
    ///
    /// * `order` - Vector of job types in priority order (first = highest)
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn type_priority(mut self, order: Vec<T>) -> Self {
        self.type_priority = order;
        self
    }

    /// Sets the worker poll interval.
    ///
    /// Controls how frequently workers check for new jobs and shutdown signals.
    ///
    /// # Panics
    ///
    /// Panics if interval is zero.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        assert!(!interval.is_zero(), "poll interval must be non-zero");
        self.poll_interval = interval;
        self
    }

    /// Sets the thread name prefix.
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_thread_name_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.thread_name_prefix = prefix.into();
        self
    }

    /// Returns the worker count for a job type.
    pub fn get_workers_for(&self, job_type: T) -> usize {
        self.workers_per_type
            .get(&job_type)
            .copied()
            .unwrap_or(self.default_workers)
    }

    /// Returns the queue capacity for a job type.
    pub fn get_queue_capacity_for(&self, job_type: T) -> usize {
        self.queue_capacity_per_type
            .get(&job_type)
            .copied()
            .unwrap_or(self.default_queue_capacity)
    }

    /// Returns the total number of workers across all types.
    pub fn total_workers(&self) -> usize {
        T::all_variants()
            .iter()
            .map(|t| self.get_workers_for(*t))
            .sum()
    }

    /// Validates the configuration.
    pub fn validate(&self) -> crate::core::Result<()> {
        for job_type in T::all_variants() {
            if self.get_workers_for(*job_type) == 0 {
                return Err(crate::core::ThreadError::invalid_config(
                    "workers_per_type",
                    format!("worker count for {:?} must be greater than 0", job_type),
                ));
            }
        }
        Ok(())
    }
}

/// Convenience presets for common workload patterns.
impl TypedPoolConfig<super::DefaultJobType> {
    /// Creates a configuration optimized for IO-heavy workloads.
    ///
    /// - IO: 2x CPU count (high concurrency for IO waits)
    /// - Compute: 1x CPU count
    /// - Critical: 4 dedicated workers
    /// - Background: 2 workers
    #[must_use]
    pub fn io_optimized() -> Self {
        let cpus = num_cpus::get();
        Self::new()
            .workers_for(super::DefaultJobType::Io, cpus * 2)
            .workers_for(super::DefaultJobType::Compute, cpus)
            .workers_for(super::DefaultJobType::Critical, 4)
            .workers_for(super::DefaultJobType::Background, 2)
            .type_priority(vec![
                super::DefaultJobType::Critical,
                super::DefaultJobType::Io,
                super::DefaultJobType::Compute,
                super::DefaultJobType::Background,
            ])
    }

    /// Creates a configuration optimized for CPU-heavy workloads.
    ///
    /// - Compute: 1x CPU count
    /// - IO: 4 workers
    /// - Critical: 4 dedicated workers
    /// - Background: 2 workers
    #[must_use]
    pub fn compute_optimized() -> Self {
        let cpus = num_cpus::get();
        Self::new()
            .workers_for(super::DefaultJobType::Compute, cpus)
            .workers_for(super::DefaultJobType::Io, 4)
            .workers_for(super::DefaultJobType::Critical, 4)
            .workers_for(super::DefaultJobType::Background, 2)
            .type_priority(vec![
                super::DefaultJobType::Critical,
                super::DefaultJobType::Compute,
                super::DefaultJobType::Io,
                super::DefaultJobType::Background,
            ])
    }

    /// Creates a balanced configuration for mixed workloads.
    ///
    /// - All types: 1x CPU count / 4 (minimum 2)
    /// - Critical has priority
    #[must_use]
    pub fn balanced() -> Self {
        let workers = (num_cpus::get() / 4).max(2);
        Self::new()
            .workers_for(super::DefaultJobType::Io, workers)
            .workers_for(super::DefaultJobType::Compute, workers)
            .workers_for(super::DefaultJobType::Critical, workers)
            .workers_for(super::DefaultJobType::Background, workers)
            .type_priority(vec![
                super::DefaultJobType::Critical,
                super::DefaultJobType::Io,
                super::DefaultJobType::Compute,
                super::DefaultJobType::Background,
            ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed::DefaultJobType;

    #[test]
    fn test_default_config() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        assert_eq!(config.default_workers, 2);
        assert_eq!(config.default_queue_capacity, 0);
        assert_eq!(config.poll_interval, Duration::from_millis(100));
    }

    #[test]
    fn test_workers_for() {
        let config = TypedPoolConfig::<DefaultJobType>::new().workers_for(DefaultJobType::Io, 16);
        assert_eq!(config.get_workers_for(DefaultJobType::Io), 16);
        assert_eq!(config.get_workers_for(DefaultJobType::Compute), 2);
    }

    #[test]
    fn test_queue_capacity_for() {
        let config = TypedPoolConfig::<DefaultJobType>::new()
            .queue_capacity_for(DefaultJobType::Critical, 1000);
        assert_eq!(
            config.get_queue_capacity_for(DefaultJobType::Critical),
            1000
        );
        assert_eq!(config.get_queue_capacity_for(DefaultJobType::Io), 0);
    }

    #[test]
    fn test_type_priority() {
        let config = TypedPoolConfig::<DefaultJobType>::new()
            .type_priority(vec![DefaultJobType::Critical, DefaultJobType::Background]);
        assert_eq!(config.type_priority.len(), 2);
        assert_eq!(config.type_priority[0], DefaultJobType::Critical);
    }

    #[test]
    fn test_total_workers() {
        let config = TypedPoolConfig::<DefaultJobType>::new()
            .workers_for(DefaultJobType::Io, 4)
            .workers_for(DefaultJobType::Compute, 8);
        // Io: 4, Compute: 8, Critical: 2 (default), Background: 2 (default)
        assert_eq!(config.total_workers(), 4 + 8 + 2 + 2);
    }

    #[test]
    fn test_validate_success() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        assert!(config.validate().is_ok());
    }

    #[test]
    #[should_panic(expected = "poll interval must be non-zero")]
    fn test_zero_poll_interval_panics() {
        let _ = TypedPoolConfig::<DefaultJobType>::new().with_poll_interval(Duration::ZERO);
    }

    #[test]
    fn test_io_optimized_preset() {
        let config = TypedPoolConfig::<DefaultJobType>::io_optimized();
        let cpus = num_cpus::get();
        assert_eq!(config.get_workers_for(DefaultJobType::Io), cpus * 2);
        assert_eq!(config.get_workers_for(DefaultJobType::Compute), cpus);
        assert_eq!(config.type_priority[0], DefaultJobType::Critical);
    }

    #[test]
    fn test_compute_optimized_preset() {
        let config = TypedPoolConfig::<DefaultJobType>::compute_optimized();
        let cpus = num_cpus::get();
        assert_eq!(config.get_workers_for(DefaultJobType::Compute), cpus);
        assert_eq!(config.get_workers_for(DefaultJobType::Io), 4);
    }

    #[test]
    fn test_balanced_preset() {
        let config = TypedPoolConfig::<DefaultJobType>::balanced();
        let workers = (num_cpus::get() / 4).max(2);
        assert_eq!(config.get_workers_for(DefaultJobType::Io), workers);
        assert_eq!(config.get_workers_for(DefaultJobType::Compute), workers);
    }
}
