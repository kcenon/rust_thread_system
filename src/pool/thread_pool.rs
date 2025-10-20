//! Thread pool implementation

use crate::core::{BoxedJob, CancellationToken, ClosureJob, Job, JobHandle, Result, ThreadError};
use crate::pool::worker::{Worker, WorkerStats};
use crossbeam::channel::{bounded, unbounded, Sender};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Configuration for thread pool
#[derive(Debug, Clone)]
pub struct ThreadPoolConfig {
    /// Number of worker threads (0 = number of CPUs)
    pub num_threads: usize,
    /// Maximum queue size (0 = unbounded)
    pub max_queue_size: usize,
    /// Thread name prefix
    pub thread_name_prefix: String,
}

impl Default for ThreadPoolConfig {
    fn default() -> Self {
        Self {
            num_threads: num_cpus::get(),
            // Default to bounded queue of 10,000 to prevent memory exhaustion
            // Use 0 or with_max_queue_size(0) for unbounded queue
            max_queue_size: 10_000,
            thread_name_prefix: "worker".to_string(),
        }
    }
}

impl ThreadPoolConfig {
    /// Create a new configuration with specified number of threads
    #[must_use]
    pub fn new(num_threads: usize) -> Self {
        Self {
            num_threads: if num_threads == 0 {
                num_cpus::get()
            } else {
                num_threads
            },
            ..Default::default()
        }
    }

    /// Set maximum queue size
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_max_queue_size(mut self, size: usize) -> Self {
        self.max_queue_size = size;
        self
    }

    /// Set thread name prefix
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_thread_name_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.thread_name_prefix = prefix.into();
        self
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.num_threads == 0 {
            return Err(ThreadError::invalid_config(
                "num_threads",
                "Number of threads must be greater than 0",
            ));
        }
        Ok(())
    }
}

/// A job wrapper that supports cancellation
struct CancellableJob<F>
where
    F: FnOnce(CancellationToken) -> Result<()> + Send,
{
    job_id: u64,
    token: CancellationToken,
    closure: Option<F>,
}

impl<F> CancellableJob<F>
where
    F: FnOnce(CancellationToken) -> Result<()> + Send,
{
    fn new(job_id: u64, token: CancellationToken, closure: F) -> Self {
        Self {
            job_id,
            token,
            closure: Some(closure),
        }
    }
}

impl<F> Job for CancellableJob<F>
where
    F: FnOnce(CancellationToken) -> Result<()> + Send + 'static,
{
    fn execute(&mut self) -> Result<()> {
        // Check if cancelled before even starting
        if self.token.is_cancelled() {
            return Err(ThreadError::cancelled(
                self.job_id.to_string(),
                "Job was cancelled before execution",
            ));
        }

        if let Some(closure) = self.closure.take() {
            closure(self.token.clone())
        } else {
            Err(ThreadError::other(
                "CancellableJob already executed - cannot execute twice",
            ))
        }
    }

    fn job_type(&self) -> &str {
        "CancellableJob"
    }

    fn is_cancellable(&self) -> bool {
        true
    }
}

/// A thread pool for executing jobs concurrently
///
/// # Shutdown Mechanism
///
/// The pool uses channel disconnection (dropping the sender) to signal workers
/// to shutdown, NOT an atomic shutdown flag. This ensures all queued jobs are
/// processed before shutdown completes.
#[derive(Debug)]
pub struct ThreadPool {
    config: ThreadPoolConfig,
    workers: RwLock<Vec<Worker>>,
    sender: Arc<parking_lot::RwLock<Option<Sender<BoxedJob>>>>,
    running: Arc<AtomicBool>,
    total_jobs_submitted: AtomicU64,
    queue_size: Arc<AtomicU64>,
}

impl ThreadPool {
    /// Create a new thread pool with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(ThreadPoolConfig::default())
    }

    /// Create a thread pool with specified number of threads
    pub fn with_threads(num_threads: usize) -> Result<Self> {
        Self::with_config(ThreadPoolConfig::new(num_threads))
    }

    /// Create a thread pool with custom configuration
    pub fn with_config(config: ThreadPoolConfig) -> Result<Self> {
        config.validate()?;

        let queue_size = Arc::new(AtomicU64::new(0));

        Ok(Self {
            config,
            workers: RwLock::new(Vec::new()),
            sender: Arc::new(parking_lot::RwLock::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            total_jobs_submitted: AtomicU64::new(0),
            queue_size,
        })
    }

    /// Start the thread pool
    ///
    /// # Restart Support
    ///
    /// The pool can be restarted after shutdown by calling start() again.
    /// Workers will be recreated with a new channel.
    pub fn start(&mut self) -> Result<()> {
        // Atomically check and set running flag to prevent race condition
        // Multiple threads calling start() simultaneously will now be serialized
        // Only the first thread will succeed, others will get AlreadyRunning error
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ThreadError::already_running(
                &self.config.thread_name_prefix,
                self.config.num_threads,
            ));
        }

        // Create channel based on configuration
        let (sender, receiver) = if self.config.max_queue_size > 0 {
            bounded(self.config.max_queue_size)
        } else {
            unbounded()
        };

        // Create workers
        let mut workers = Vec::with_capacity(self.config.num_threads);
        for id in 0..self.config.num_threads {
            let worker = Worker::new(id, receiver.clone(), Arc::clone(&self.queue_size))?;
            workers.push(worker);
        }

        *self.workers.write() = workers;
        *self.sender.write() = Some(sender);

        // Running flag is already set by compare_exchange above

        Ok(())
    }

    /// Submit a job to the pool
    pub fn submit<J: Job + 'static>(&self, job: J) -> Result<()> {
        // Check if pool is running - this is the primary gate for job submission
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        // Atomically get sender reference - no TOCTOU race condition
        // Even if shutdown() is called between the running check and here,
        // we either get a valid sender or None - both are handled correctly
        let sender_guard = self.sender.read();
        let sender = sender_guard
            .as_ref()
            .ok_or_else(|| ThreadError::not_running(&self.config.thread_name_prefix))?;

        // Check for queue size overflow before incrementing
        // Extremely unlikely (would require 2^64 jobs submitted without processing)
        // but we protect against it to prevent silent wraparound
        let current_queue_size = self.queue_size.load(Ordering::Relaxed);
        if current_queue_size == u64::MAX {
            return Err(ThreadError::other(
                "Queue size counter overflow - this should never happen in practice",
            ));
        }

        // Increment queue size before sending
        self.queue_size.fetch_add(1, Ordering::Relaxed);

        // Send the job - sender is valid for duration of this scope
        let result = sender
            .send(Box::new(job))
            .map_err(|_| ThreadError::shutting_down(0));

        if result.is_err() {
            // Failed to send, decrement queue size
            self.queue_size.fetch_sub(1, Ordering::Relaxed);
            return result;
        }

        self.total_jobs_submitted.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Submit a closure as a job
    pub fn execute<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.submit(ClosureJob::new(f))
    }

    /// Submit a cancellable closure and get a handle to control it
    ///
    /// Returns a `JobHandle` that can be used to cancel the job.
    /// The job must cooperatively check the cancellation token
    /// to actually respond to cancellation requests.
    ///
    /// # Example
    ///
    /// ```
    /// use rust_thread_system::prelude::*;
    /// use std::time::Duration;
    ///
    /// # fn main() -> Result<()> {
    /// let mut pool = ThreadPool::with_threads(2)?;
    /// pool.start()?;
    ///
    /// let handle = pool.submit_cancellable(|token| {
    ///     for i in 0..10 {
    ///         // Check for cancellation
    ///         if token.is_cancelled() {
    ///             return Err(ThreadError::cancelled("job-1", "Cancelled by user"));
    ///         }
    ///         std::thread::sleep(Duration::from_millis(100));
    ///     }
    ///     Ok(())
    /// })?;
    ///
    /// // Cancel the job
    /// handle.cancel();
    /// # pool.shutdown()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn submit_cancellable<F>(&self, f: F) -> Result<JobHandle>
    where
        F: FnOnce(CancellationToken) -> Result<()> + Send + 'static,
    {
        let handle = JobHandle::new();
        let token = handle.token().clone();
        let job_id = handle.job_id();

        // Create a wrapper job that checks cancellation before execution
        let job = CancellableJob::new(job_id, token.clone(), f);

        self.submit(job)?;

        Ok(handle)
    }

    /// Get the number of worker threads
    pub fn num_threads(&self) -> usize {
        self.config.num_threads
    }

    /// Check if the pool is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Get total number of jobs submitted
    pub fn total_jobs_submitted(&self) -> u64 {
        self.total_jobs_submitted.load(Ordering::Relaxed)
    }

    /// Get current queue size (approximate)
    ///
    /// This represents the number of jobs waiting to be processed.
    /// The value is approximate as it may change between checking and using it.
    pub fn queue_size(&self) -> u64 {
        self.queue_size.load(Ordering::Relaxed)
    }

    /// Get statistics for all workers
    pub fn get_stats(&self) -> Vec<Arc<WorkerStats>> {
        self.workers.read().iter().map(|w| w.stats()).collect()
    }

    /// Get total jobs processed across all workers
    pub fn total_jobs_processed(&self) -> u64 {
        // Optimized: acquire lock once and iterate directly
        let workers = self.workers.read();
        workers.iter().map(|w| w.stats().get_jobs_processed()).sum()
    }

    /// Get total jobs failed across all workers
    pub fn total_jobs_failed(&self) -> u64 {
        // Optimized: acquire lock once and iterate directly
        let workers = self.workers.read();
        workers.iter().map(|w| w.stats().get_jobs_failed()).sum()
    }

    /// Get total jobs panicked across all workers
    pub fn total_jobs_panicked(&self) -> u64 {
        // Optimized: acquire lock once and iterate directly
        let workers = self.workers.read();
        workers.iter().map(|w| w.stats().get_jobs_panicked()).sum()
    }

    /// Shutdown the thread pool and wait for all workers to finish
    ///
    /// # Graceful Shutdown
    ///
    /// 1. Stops accepting new jobs (sets running = false)
    /// 2. Closes the channel (drops sender)
    /// 3. Waits for all workers to drain queued jobs and exit
    ///
    /// This ensures all queued jobs are processed before shutdown completes.
    pub fn shutdown(&mut self) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Ok(());
        }

        // Mark as not running first to prevent new job submissions
        self.running.store(false, Ordering::Release);

        // Drop sender to close the channel
        // Thread-safe: acquires write lock, preventing concurrent submit() from using stale sender
        *self.sender.write() = None;

        // Wait for all workers to finish draining the queue
        // Workers will exit when they receive RecvTimeoutError::Disconnected
        let workers = std::mem::take(&mut *self.workers.write());
        for worker in workers {
            worker.join()?;
        }

        Ok(())
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        // Only attempt shutdown if still running to avoid redundant work
        if self.running.load(Ordering::Acquire) {
            if let Err(e) = self.shutdown() {
                eprintln!(
                    "[THREAD_POOL ERROR] Failed to shutdown thread pool '{}' during drop: {}",
                    self.config.thread_name_prefix, e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_thread_pool_creation() {
        let mut pool = ThreadPool::new().expect("Failed to create thread pool");
        assert!(!pool.is_running());

        pool.start().expect("Failed to start pool");
        assert!(pool.is_running());
        assert_eq!(pool.num_threads(), num_cpus::get());

        pool.shutdown().expect("Failed to shutdown pool");
        assert!(!pool.is_running());
    }

    #[test]
    fn test_thread_pool_with_threads() {
        let mut pool = ThreadPool::with_threads(4).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");
        assert_eq!(pool.num_threads(), 4);
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_job_execution() {
        let mut pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit jobs
        for _ in 0..10 {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .expect("Failed to submit job");
        }

        // Wait for jobs to complete
        thread::sleep(Duration::from_millis(100));

        assert_eq!(counter.load(Ordering::Relaxed), 10);
        assert_eq!(pool.total_jobs_submitted(), 10);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_bounded_queue() {
        let config = ThreadPoolConfig::new(2).with_max_queue_size(5);
        let mut pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Submit jobs until queue is full
        let mut submitted = 0;
        for _ in 0..100 {
            if pool
                .execute(|| {
                    thread::sleep(Duration::from_millis(10));
                    Ok(())
                })
                .is_ok()
            {
                submitted += 1;
            } else {
                break;
            }
        }

        // Should be able to submit at least a few jobs
        assert!(submitted > 0);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_submit_when_not_running() {
        let pool = ThreadPool::new().expect("Failed to create thread pool");
        let result = pool.execute(|| Ok(()));
        assert!(matches!(result, Err(ThreadError::NotRunning { .. })));
    }

    #[test]
    fn test_stress_high_load() {
        let mut pool = ThreadPool::with_threads(4).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let jobs_count = 10000;

        // Submit large number of jobs
        for _ in 0..jobs_count {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                // Simulate some work
                thread::sleep(Duration::from_micros(10));
                Ok(())
            })
            .expect("Failed to submit job");
        }

        // Wait for all jobs to complete
        thread::sleep(Duration::from_secs(3));

        assert_eq!(counter.load(Ordering::Relaxed), jobs_count);
        assert_eq!(pool.total_jobs_submitted(), jobs_count as u64);
        assert_eq!(pool.total_jobs_processed(), jobs_count as u64);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_concurrent_submit() {
        use std::sync::Arc;

        let mut pool = ThreadPool::with_threads(4).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");
        let pool = Arc::new(pool);

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        // Spawn multiple threads that submit jobs concurrently
        for _ in 0..10 {
            let pool_clone = Arc::clone(&pool);
            let counter_clone = Arc::clone(&counter);

            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    let counter_inner = Arc::clone(&counter_clone);
                    let _ = pool_clone.execute(move || {
                        counter_inner.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    });
                }
            });
            handles.push(handle);
        }

        // Wait for all submission threads
        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        // Wait for all jobs to complete
        thread::sleep(Duration::from_millis(500));

        assert_eq!(counter.load(Ordering::Relaxed), 1000);
        assert_eq!(pool.total_jobs_submitted(), 1000);

        // Note: can't call shutdown on Arc<ThreadPool>
        // It will be dropped automatically
    }

    #[test]
    fn test_shutdown_waits_for_jobs() {
        let mut pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit fast jobs that will complete quickly
        for _ in 0..10 {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .expect("Failed to submit job");
        }

        // Let workers start processing
        thread::sleep(Duration::from_millis(200));

        // Shutdown - should wait for remaining jobs to complete
        pool.shutdown().expect("Failed to shutdown pool");

        // Verify shutdown completed gracefully
        assert!(!pool.is_running());

        // All jobs should have run
        assert_eq!(counter.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn test_submit_after_shutdown() {
        let mut pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Submit a job
        pool.execute(|| Ok(())).expect("Failed to submit job");

        // Shutdown
        pool.shutdown().expect("Failed to shutdown pool");

        // Try to submit after shutdown
        let result = pool.execute(|| Ok(()));
        assert!(matches!(result, Err(ThreadError::NotRunning { .. })));
    }

    #[test]
    fn test_error_handling() {
        let mut pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit jobs that fail
        for i in 0..10 {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                if i % 2 == 0 {
                    Err(ThreadError::other("Test error"))
                } else {
                    Ok(())
                }
            })
            .expect("Failed to submit job");
        }

        // Wait for jobs to complete
        thread::sleep(Duration::from_millis(100));

        // All jobs should have been attempted
        assert_eq!(counter.load(Ordering::Relaxed), 10);
        assert_eq!(pool.total_jobs_submitted(), 10);

        // 5 should have succeeded, 5 failed
        assert_eq!(pool.total_jobs_processed(), 5);
        assert_eq!(pool.total_jobs_failed(), 5);

        pool.shutdown().expect("Failed to shutdown pool");
    }
}
