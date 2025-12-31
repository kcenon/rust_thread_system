//! Typed thread pool implementation.
//!
//! This module provides [`TypedThreadPool`] for type-based job routing
//! with per-type queues and workers.

use super::{
    AtomicTypeStats, JobType, TypeStats, TypedClosureJob, TypedJob, TypedPoolConfig, TypedWorker,
};
use crate::core::{Result, ThreadError};
use crate::queue::{BoundedQueue, ChannelQueue, JobQueue, QueueError};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// A typed thread pool that routes jobs to dedicated queues based on job type.
///
/// This pool enables QoS guarantees and type-specific scheduling by maintaining
/// separate queues for each job type.
///
/// # Features
///
/// - **Per-type queues**: Each job type has its own dedicated queue
/// - **Per-type workers**: Configurable worker counts per job type
/// - **Priority scheduling**: Workers check queues in configured priority order
/// - **Work stealing**: Idle workers can steal from other type queues
/// - **Per-type statistics**: Track metrics per job type
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::typed::{TypedThreadPool, TypedPoolConfig, DefaultJobType};
///
/// let config = TypedPoolConfig::<DefaultJobType>::new()
///     .workers_for(DefaultJobType::Critical, 4)
///     .workers_for(DefaultJobType::Compute, 8)
///     .workers_for(DefaultJobType::Io, 16)
///     .workers_for(DefaultJobType::Background, 2);
///
/// let pool = TypedThreadPool::new(config)?;
/// pool.start()?;
///
/// // Submit jobs with explicit types
/// pool.execute_typed(DefaultJobType::Critical, || {
///     println!("Critical task");
///     Ok(())
/// })?;
///
/// pool.shutdown()?;
/// ```
pub struct TypedThreadPool<T: JobType> {
    config: TypedPoolConfig<T>,
    queues: RwLock<HashMap<T, Arc<dyn JobQueue>>>,
    workers: RwLock<Vec<TypedWorker<T>>>,
    stats: Arc<HashMap<T, Arc<AtomicTypeStats>>>,
    running: Arc<AtomicBool>,
    total_jobs_submitted: AtomicU64,
}

impl<T: JobType> std::fmt::Debug for TypedThreadPool<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedThreadPool")
            .field("config", &self.config)
            .field("running", &self.running.load(Ordering::Relaxed))
            .field(
                "total_jobs_submitted",
                &self.total_jobs_submitted.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl<T: JobType> TypedThreadPool<T> {
    /// Creates a new typed thread pool with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn new(config: TypedPoolConfig<T>) -> Result<Self> {
        config.validate()?;

        // Initialize per-type statistics
        let mut stats_map = HashMap::new();
        for job_type in T::all_variants() {
            stats_map.insert(*job_type, Arc::new(AtomicTypeStats::new()));
        }

        Ok(Self {
            config,
            queues: RwLock::new(HashMap::new()),
            workers: RwLock::new(Vec::new()),
            stats: Arc::new(stats_map),
            running: Arc::new(AtomicBool::new(false)),
            total_jobs_submitted: AtomicU64::new(0),
        })
    }

    /// Starts the thread pool.
    ///
    /// Creates queues for each job type and spawns workers according to
    /// the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the pool is already running or if worker
    /// threads fail to spawn.
    pub fn start(&self) -> Result<()> {
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ThreadError::already_running(
                &self.config.thread_name_prefix,
                self.config.total_workers(),
            ));
        }

        // Create queues for each job type
        let mut queues = HashMap::new();
        for job_type in T::all_variants() {
            let capacity = self.config.get_queue_capacity_for(*job_type);
            let queue: Arc<dyn JobQueue> = if capacity > 0 {
                Arc::new(BoundedQueue::new(capacity))
            } else {
                Arc::new(ChannelQueue::unbounded())
            };
            queues.insert(*job_type, queue);
        }

        // Build priority-ordered queue list
        let all_queues: Vec<(T, Arc<dyn JobQueue>)> = self
            .config
            .type_priority
            .iter()
            .filter_map(|t| queues.get(t).map(|q| (*t, Arc::clone(q))))
            .collect();

        // Create workers for each job type
        let mut workers = Vec::new();
        let mut worker_id = 0;

        for job_type in T::all_variants() {
            let queue = queues
                .get(job_type)
                .ok_or_else(|| ThreadError::other(format!("Queue not found for {:?}", job_type)))?;

            let worker_count = self.config.get_workers_for(*job_type);
            for _ in 0..worker_count {
                let worker = TypedWorker::new(
                    worker_id,
                    *job_type,
                    Arc::clone(queue),
                    all_queues.clone(),
                    Arc::clone(&self.stats),
                    Arc::clone(&self.running),
                    self.config.poll_interval,
                    &self.config.thread_name_prefix,
                )?;
                workers.push(worker);
                worker_id += 1;
            }
        }

        *self.queues.write() = queues;
        *self.workers.write() = workers;

        Ok(())
    }

    /// Submits a typed job to the pool.
    ///
    /// The job is routed to the queue for its declared type.
    ///
    /// # Type Parameters
    ///
    /// * `J` - Job type implementing [`TypedJob<T>`]
    ///
    /// # Errors
    ///
    /// Returns an error if the pool is not running or the queue is closed.
    pub fn submit<J>(&self, job: J) -> Result<()>
    where
        J: TypedJob<T> + 'static,
    {
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        let job_type = job.typed_job_type();
        let queues = self.queues.read();
        let queue = queues
            .get(&job_type)
            .ok_or_else(|| ThreadError::other(format!("No queue for job type {:?}", job_type)))?;

        // Record submission
        if let Some(stat) = self.stats.get(&job_type) {
            stat.record_submission();
        }

        queue.send(Box::new(job)).map_err(|e| match e {
            QueueError::Closed(_) => ThreadError::shutting_down(0),
            QueueError::Full(_) => ThreadError::queue_full(queue.len(), 0),
            _ => ThreadError::QueueSendError,
        })?;

        self.total_jobs_submitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Executes a closure with the specified job type.
    ///
    /// # Arguments
    ///
    /// * `job_type` - The type category for routing
    /// * `f` - The closure to execute
    ///
    /// # Errors
    ///
    /// Returns an error if the pool is not running or the queue is closed.
    pub fn execute_typed<F>(&self, job_type: T, f: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.submit(TypedClosureJob::new(job_type, f))
    }

    /// Executes a closure with the default job type.
    ///
    /// Uses [`JobType::default_type()`] for routing.
    ///
    /// # Errors
    ///
    /// Returns an error if the pool is not running or the queue is closed.
    pub fn execute<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.execute_typed(T::default_type(), f)
    }

    /// Attempts to submit a typed job without blocking.
    ///
    /// Returns immediately if the queue is full.
    pub fn try_submit<J>(&self, job: J) -> Result<()>
    where
        J: TypedJob<T> + 'static,
    {
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        let job_type = job.typed_job_type();
        let queues = self.queues.read();
        let queue = queues
            .get(&job_type)
            .ok_or_else(|| ThreadError::other(format!("No queue for job type {:?}", job_type)))?;

        if let Some(stat) = self.stats.get(&job_type) {
            stat.record_submission();
        }

        queue.try_send(Box::new(job)).map_err(|e| match e {
            QueueError::Closed(_) => ThreadError::shutting_down(0),
            QueueError::Full(_) => ThreadError::queue_full(queue.len(), 0),
            _ => ThreadError::QueueSendError,
        })?;

        self.total_jobs_submitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Attempts to execute a typed closure without blocking.
    pub fn try_execute_typed<F>(&self, job_type: T, f: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.try_submit(TypedClosureJob::new(job_type, f))
    }

    /// Returns whether the pool is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Returns the total number of jobs submitted.
    pub fn total_jobs_submitted(&self) -> u64 {
        self.total_jobs_submitted.load(Ordering::Relaxed)
    }

    /// Returns statistics for a specific job type.
    pub fn type_stats(&self, job_type: T) -> Option<TypeStats> {
        let queues = self.queues.read();
        let queue_depth = queues.get(&job_type).map(|q| q.len()).unwrap_or(0);
        self.stats.get(&job_type).map(|s| s.snapshot(queue_depth))
    }

    /// Returns statistics for all job types.
    pub fn all_stats(&self) -> HashMap<T, TypeStats> {
        let queues = self.queues.read();
        self.stats
            .iter()
            .map(|(t, s)| {
                let queue_depth = queues.get(t).map(|q| q.len()).unwrap_or(0);
                (*t, s.snapshot(queue_depth))
            })
            .collect()
    }

    /// Returns the current queue depth for a job type.
    pub fn queue_depth(&self, job_type: T) -> usize {
        self.queues
            .read()
            .get(&job_type)
            .map(|q| q.len())
            .unwrap_or(0)
    }

    /// Returns the total queue depth across all job types.
    pub fn total_queue_depth(&self) -> usize {
        self.queues.read().values().map(|q| q.len()).sum()
    }

    /// Returns the total number of workers.
    pub fn num_workers(&self) -> usize {
        self.workers.read().len()
    }

    /// Returns the configuration.
    pub fn config(&self) -> &TypedPoolConfig<T> {
        &self.config
    }

    /// Shuts down the thread pool.
    ///
    /// Stops accepting new jobs and waits for all workers to finish
    /// processing queued jobs.
    pub fn shutdown(&self) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Ok(());
        }

        // Mark as not running to prevent new submissions
        self.running.store(false, Ordering::Release);

        // Close all queues
        for queue in self.queues.read().values() {
            queue.close();
        }

        // Wait for all workers to finish
        let workers = std::mem::take(&mut *self.workers.write());
        for worker in workers {
            worker.join()?;
        }

        // Clear queues
        self.queues.write().clear();

        Ok(())
    }
}

impl<T: JobType> Drop for TypedThreadPool<T> {
    fn drop(&mut self) {
        if self.running.load(Ordering::Acquire) {
            if let Err(e) = self.shutdown() {
                eprintln!(
                    "[TYPED_POOL ERROR] Failed to shutdown typed thread pool during drop: {}",
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed::DefaultJobType;
    use std::sync::atomic::AtomicUsize;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_pool_creation() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        assert!(!pool.is_running());
    }

    #[test]
    fn test_pool_start_and_shutdown() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");

        pool.start().expect("Failed to start pool");
        assert!(pool.is_running());

        pool.shutdown().expect("Failed to shutdown pool");
        assert!(!pool.is_running());
    }

    #[test]
    fn test_pool_double_start_fails() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");

        pool.start().expect("Failed to start pool");
        assert!(pool.start().is_err());

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_execute_typed() {
        let config = TypedPoolConfig::<DefaultJobType>::new().workers_for(DefaultJobType::Io, 2);
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        pool.execute_typed(DefaultJobType::Io, move || {
            counter_clone.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .expect("Failed to execute job");

        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        let stats = pool.type_stats(DefaultJobType::Io).unwrap();
        assert_eq!(stats.jobs_submitted, 1);
        assert_eq!(stats.jobs_completed, 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_execute_default_type() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        pool.execute(move || {
            counter_clone.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .expect("Failed to execute job");

        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_multiple_types() {
        let config = TypedPoolConfig::<DefaultJobType>::new()
            .workers_for(DefaultJobType::Io, 2)
            .workers_for(DefaultJobType::Compute, 2)
            .workers_for(DefaultJobType::Critical, 2)
            .workers_for(DefaultJobType::Background, 1);

        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        let io_counter = Arc::new(AtomicUsize::new(0));
        let compute_counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..10 {
            let io_clone = Arc::clone(&io_counter);
            pool.execute_typed(DefaultJobType::Io, move || {
                io_clone.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .expect("Failed to submit IO job");

            let compute_clone = Arc::clone(&compute_counter);
            pool.execute_typed(DefaultJobType::Compute, move || {
                compute_clone.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .expect("Failed to submit Compute job");
        }

        thread::sleep(Duration::from_millis(200));

        assert_eq!(io_counter.load(Ordering::Relaxed), 10);
        assert_eq!(compute_counter.load(Ordering::Relaxed), 10);

        let io_stats = pool.type_stats(DefaultJobType::Io).unwrap();
        let compute_stats = pool.type_stats(DefaultJobType::Compute).unwrap();

        assert_eq!(io_stats.jobs_submitted, 10);
        assert_eq!(io_stats.jobs_completed, 10);
        assert_eq!(compute_stats.jobs_submitted, 10);
        assert_eq!(compute_stats.jobs_completed, 10);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_all_stats() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        pool.execute_typed(DefaultJobType::Io, || Ok(()))
            .expect("Failed to submit job");
        pool.execute_typed(DefaultJobType::Compute, || Ok(()))
            .expect("Failed to submit job");

        thread::sleep(Duration::from_millis(100));

        let all_stats = pool.all_stats();
        assert_eq!(all_stats.len(), 4); // All 4 types

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_submit_when_not_running() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");

        let result = pool.execute(|| Ok(()));
        assert!(matches!(result, Err(ThreadError::NotRunning { .. })));
    }

    #[test]
    fn test_total_jobs_submitted() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        for _ in 0..5 {
            pool.execute(|| Ok(())).expect("Failed to submit job");
        }

        assert_eq!(pool.total_jobs_submitted(), 5);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_num_workers() {
        let config = TypedPoolConfig::<DefaultJobType>::new()
            .workers_for(DefaultJobType::Io, 4)
            .workers_for(DefaultJobType::Compute, 8)
            .workers_for(DefaultJobType::Critical, 2)
            .workers_for(DefaultJobType::Background, 1);

        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        assert_eq!(pool.num_workers(), 4 + 8 + 2 + 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_presets() {
        let io_config = TypedPoolConfig::<DefaultJobType>::io_optimized();
        let pool = TypedThreadPool::new(io_config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");
        pool.shutdown().expect("Failed to shutdown pool");

        let compute_config = TypedPoolConfig::<DefaultJobType>::compute_optimized();
        let pool = TypedThreadPool::new(compute_config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");
        pool.shutdown().expect("Failed to shutdown pool");

        let balanced_config = TypedPoolConfig::<DefaultJobType>::balanced();
        let pool = TypedThreadPool::new(balanced_config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_job_failure_tracking() {
        let config = TypedPoolConfig::<DefaultJobType>::new();
        let pool = TypedThreadPool::new(config).expect("Failed to create pool");
        pool.start().expect("Failed to start pool");

        pool.execute_typed(DefaultJobType::Compute, || {
            Err(ThreadError::other("Intentional failure"))
        })
        .expect("Failed to submit job");

        thread::sleep(Duration::from_millis(100));

        let stats = pool.type_stats(DefaultJobType::Compute).unwrap();
        assert_eq!(stats.jobs_failed, 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }
}
