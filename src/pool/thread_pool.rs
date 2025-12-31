//! Thread pool implementation

#[cfg(feature = "priority-scheduling")]
use crate::core::Priority;
use crate::core::{CancellationToken, ClosureJob, Job, JobHandle, Result, ThreadError};
use crate::pool::worker::{Worker, WorkerStats};
#[cfg(feature = "priority-scheduling")]
use crate::queue::PriorityJobQueue;
use crate::queue::{BackpressureStrategy, BoundedQueue, ChannelQueue, JobQueue, QueueError};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Configuration for thread pool
#[derive(Clone)]
pub struct ThreadPoolConfig {
    /// Number of worker threads (0 = number of CPUs)
    pub num_threads: usize,
    /// Maximum queue size (0 = unbounded)
    pub max_queue_size: usize,
    /// Thread name prefix
    pub thread_name_prefix: String,
    /// Worker poll interval for checking new jobs and shutdown state.
    /// Default: 100ms
    ///
    /// Shorter intervals improve responsiveness but increase CPU usage.
    /// Longer intervals reduce CPU usage but increase shutdown latency.
    pub poll_interval: Duration,
    /// Custom queue implementation (if None, uses default based on max_queue_size)
    queue: Option<Arc<dyn JobQueue>>,
    /// Backpressure strategy for bounded queues.
    /// Default: Block
    pub backpressure_strategy: BackpressureStrategy,
    /// Enable priority scheduling (requires `priority-scheduling` feature).
    /// When enabled, uses PriorityJobQueue instead of the default queue.
    /// Default: false
    #[cfg(feature = "priority-scheduling")]
    pub enable_priority: bool,
}

impl std::fmt::Debug for ThreadPoolConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("ThreadPoolConfig");
        debug
            .field("num_threads", &self.num_threads)
            .field("max_queue_size", &self.max_queue_size)
            .field("thread_name_prefix", &self.thread_name_prefix)
            .field("poll_interval", &self.poll_interval)
            .field("queue", &self.queue.as_ref().map(|_| "<custom queue>"))
            .field("backpressure_strategy", &self.backpressure_strategy);
        #[cfg(feature = "priority-scheduling")]
        debug.field("enable_priority", &self.enable_priority);
        debug.finish()
    }
}

impl Default for ThreadPoolConfig {
    fn default() -> Self {
        Self {
            num_threads: num_cpus::get(),
            // Default to bounded queue of 10,000 to prevent memory exhaustion
            // Use 0 or with_max_queue_size(0) for unbounded queue
            max_queue_size: 10_000,
            thread_name_prefix: "worker".to_string(),
            poll_interval: Duration::from_millis(100),
            queue: None,
            backpressure_strategy: BackpressureStrategy::default(),
            #[cfg(feature = "priority-scheduling")]
            enable_priority: false,
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

    /// Set the worker poll interval.
    ///
    /// This controls how frequently workers check for new jobs and shutdown signals.
    ///
    /// # Arguments
    ///
    /// * `interval` - Duration between poll attempts. Must be non-zero.
    ///
    /// # Trade-offs
    ///
    /// - **Shorter intervals** (10-50ms): Better responsiveness, faster shutdown, higher CPU usage
    /// - **Longer intervals** (500ms-1s): Lower CPU usage, slower shutdown, reduced responsiveness
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

    /// Set the backpressure strategy for bounded queues.
    ///
    /// This controls how the pool handles job submissions when the queue is full.
    ///
    /// # Arguments
    ///
    /// * `strategy` - The backpressure strategy to use
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::prelude::*;
    /// use rust_thread_system::queue::BackpressureStrategy;
    /// use std::time::Duration;
    ///
    /// // Reject immediately for real-time systems
    /// let config = ThreadPoolConfig::default()
    ///     .with_max_queue_size(1000)
    ///     .with_backpressure_strategy(BackpressureStrategy::RejectImmediately);
    ///
    /// // Timeout for web servers
    /// let config = ThreadPoolConfig::default()
    ///     .with_max_queue_size(5000)
    ///     .with_backpressure_strategy(BackpressureStrategy::BlockWithTimeout(Duration::from_secs(5)));
    /// ```
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_backpressure_strategy(mut self, strategy: BackpressureStrategy) -> Self {
        self.backpressure_strategy = strategy;
        self
    }

    /// Configure the pool to reject jobs immediately when the queue is full.
    ///
    /// This is a convenience method equivalent to:
    /// ```rust,ignore
    /// config.with_backpressure_strategy(BackpressureStrategy::RejectImmediately)
    /// ```
    ///
    /// # Use Cases
    ///
    /// - Real-time systems where waiting is not acceptable
    /// - Systems that need immediate feedback on queue capacity
    /// - Load shedding scenarios
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn reject_when_full(self) -> Self {
        self.with_backpressure_strategy(BackpressureStrategy::RejectImmediately)
    }

    /// Configure the pool to block with a timeout when the queue is full.
    ///
    /// This is a convenience method equivalent to:
    /// ```rust,ignore
    /// config.with_backpressure_strategy(BackpressureStrategy::BlockWithTimeout(timeout))
    /// ```
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time to wait for queue space
    ///
    /// # Use Cases
    ///
    /// - Web servers with request timeouts
    /// - Systems that can tolerate brief delays but need bounded latency
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn block_with_timeout(self, timeout: Duration) -> Self {
        self.with_backpressure_strategy(BackpressureStrategy::BlockWithTimeout(timeout))
    }

    /// Set a custom queue implementation.
    ///
    /// When a custom queue is provided, `max_queue_size` is ignored as the
    /// queue's behavior is determined by its implementation.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::prelude::*;
    /// use std::sync::Arc;
    ///
    /// // Use a priority queue
    /// let queue = Arc::new(PriorityJobQueue::new());
    /// let config = ThreadPoolConfig::new(4).with_queue(queue);
    /// let pool = ThreadPool::with_config(config)?;
    /// ```
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn with_queue(mut self, queue: Arc<dyn JobQueue>) -> Self {
        self.queue = Some(queue);
        self
    }

    /// Enable priority scheduling.
    ///
    /// When enabled, the pool uses a priority-based queue that processes
    /// higher priority jobs first. Jobs can be submitted with specific priorities
    /// using [`ThreadPool::submit_with_priority()`] or [`ThreadPool::execute_with_priority()`].
    ///
    /// Jobs submitted via regular [`ThreadPool::submit()`] will use `Priority::Normal`.
    ///
    /// # Note
    ///
    /// When `enable_priority` is true, `max_queue_size` is ignored as
    /// `PriorityJobQueue` is unbounded.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::prelude::*;
    ///
    /// let config = ThreadPoolConfig::new(4).enable_priority(true);
    /// let pool = ThreadPool::with_config(config)?;
    /// pool.start()?;
    ///
    /// // Submit with priority
    /// pool.execute_with_priority(|| {
    ///     println!("Critical task");
    ///     Ok(())
    /// }, Priority::Critical)?;
    /// ```
    #[cfg(feature = "priority-scheduling")]
    #[must_use = "builder methods return a new value and do not modify the original"]
    pub fn enable_priority(mut self, enable: bool) -> Self {
        self.enable_priority = enable;
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
/// The pool uses queue closing to signal workers to shutdown.
/// This ensures all queued jobs are processed before shutdown completes.
///
/// # Custom Queues
///
/// You can provide a custom queue implementation via `ThreadPoolConfig::with_queue()`.
/// This enables priority scheduling, custom backpressure, or other queue behaviors.
pub struct ThreadPool {
    config: ThreadPoolConfig,
    workers: RwLock<Vec<Worker>>,
    queue: RwLock<Option<Arc<dyn JobQueue>>>,
    running: Arc<AtomicBool>,
    total_jobs_submitted: AtomicU64,
}

impl std::fmt::Debug for ThreadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadPool")
            .field("config", &self.config)
            .field("running", &self.running.load(Ordering::Relaxed))
            .field(
                "total_jobs_submitted",
                &self.total_jobs_submitted.load(Ordering::Relaxed),
            )
            .finish()
    }
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

        Ok(Self {
            config,
            workers: RwLock::new(Vec::new()),
            queue: RwLock::new(None),
            running: Arc::new(AtomicBool::new(false)),
            total_jobs_submitted: AtomicU64::new(0),
        })
    }

    /// Start the thread pool
    ///
    /// # Restart Support
    ///
    /// The pool can be restarted after shutdown by calling start() again.
    /// Workers will be recreated with a new queue.
    ///
    /// # Thread Safety
    ///
    /// This method uses interior mutability and can be called from `&self`.
    /// Multiple concurrent calls are safe - only the first will succeed,
    /// others will receive an `AlreadyRunning` error.
    pub fn start(&self) -> Result<()> {
        // Atomically check and set running flag to prevent race condition
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

        // Create or use provided queue
        let queue: Arc<dyn JobQueue> = match &self.config.queue {
            Some(q) => Arc::clone(q),
            None => {
                #[cfg(feature = "priority-scheduling")]
                if self.config.enable_priority {
                    Arc::new(PriorityJobQueue::new())
                } else if self.config.max_queue_size > 0 {
                    Arc::new(BoundedQueue::new(self.config.max_queue_size))
                } else {
                    Arc::new(ChannelQueue::unbounded())
                }
                #[cfg(not(feature = "priority-scheduling"))]
                if self.config.max_queue_size > 0 {
                    Arc::new(BoundedQueue::new(self.config.max_queue_size))
                } else {
                    Arc::new(ChannelQueue::unbounded())
                }
            }
        };

        // Create workers
        let mut workers = Vec::with_capacity(self.config.num_threads);
        for id in 0..self.config.num_threads {
            let worker = Worker::new(id, Arc::clone(&queue), self.config.poll_interval)?;
            workers.push(worker);
        }

        *self.workers.write() = workers;
        *self.queue.write() = Some(queue);

        Ok(())
    }

    /// Submit a job to the pool
    ///
    /// The submission behavior depends on the configured [`BackpressureStrategy`]:
    ///
    /// - [`Block`](BackpressureStrategy::Block): Wait indefinitely for queue space (default)
    /// - [`BlockWithTimeout`](BackpressureStrategy::BlockWithTimeout): Wait up to the timeout
    /// - [`RejectImmediately`](BackpressureStrategy::RejectImmediately): Return error if queue is full
    /// - [`DropOldest`](BackpressureStrategy::DropOldest): Not supported by standard queue, falls back to block
    /// - [`DropNewest`](BackpressureStrategy::DropNewest): Silently drop if queue is full
    /// - [`Custom`](BackpressureStrategy::Custom): Delegate to custom handler
    pub fn submit<J: Job + 'static>(&self, job: J) -> Result<()> {
        self.submit_with_backpressure(Box::new(job))
    }

    /// Internal method to submit a boxed job with backpressure handling
    fn submit_with_backpressure(&self, job: crate::core::BoxedJob) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        let queue_guard = self.queue.read();
        let queue = queue_guard
            .as_ref()
            .ok_or_else(|| ThreadError::not_running(&self.config.thread_name_prefix))?;

        let result = match &self.config.backpressure_strategy {
            BackpressureStrategy::Block => queue.send(job).map_err(|e| match e {
                QueueError::Closed(_) => ThreadError::shutting_down(0),
                QueueError::Full(_) => {
                    ThreadError::queue_full(queue.len(), self.config.max_queue_size)
                }
                _ => ThreadError::QueueSendError,
            }),

            BackpressureStrategy::BlockWithTimeout(timeout) => {
                queue.send_timeout(job, *timeout).map_err(|e| match e {
                    QueueError::Timeout(_) => {
                        ThreadError::submission_timeout(timeout.as_millis() as u64)
                    }
                    QueueError::Closed(_) => ThreadError::shutting_down(0),
                    QueueError::Full(_) => {
                        ThreadError::queue_full(queue.len(), self.config.max_queue_size)
                    }
                    _ => ThreadError::QueueSendError,
                })
            }

            BackpressureStrategy::RejectImmediately => queue.try_send(job).map_err(|e| match e {
                QueueError::Full(_) => {
                    ThreadError::queue_full(queue.len(), self.config.max_queue_size)
                }
                QueueError::Closed(_) => ThreadError::shutting_down(0),
                _ => ThreadError::QueueSendError,
            }),

            BackpressureStrategy::DropOldest => {
                // DropOldest requires special queue support (deque-based)
                // For now, fall back to Block behavior
                // TODO: Implement DroppableQueue for full DropOldest support
                queue.send(job).map_err(|e| match e {
                    QueueError::Closed(_) => ThreadError::shutting_down(0),
                    QueueError::Full(_) => {
                        ThreadError::queue_full(queue.len(), self.config.max_queue_size)
                    }
                    _ => ThreadError::QueueSendError,
                })
            }

            BackpressureStrategy::DropNewest => {
                // Try to send, silently drop if queue is full
                match queue.try_send(job) {
                    Ok(()) => Ok(()),
                    Err(QueueError::Full(_)) => {
                        // Silently drop the job
                        Ok(())
                    }
                    Err(QueueError::Closed(_)) => Err(ThreadError::shutting_down(0)),
                    Err(_) => Err(ThreadError::QueueSendError),
                }
            }

            BackpressureStrategy::Custom(handler) => {
                // First try to send normally
                match queue.try_send(job) {
                    Ok(()) => Ok(()),
                    Err(QueueError::Full(holder)) => {
                        // Get the job back from the holder
                        if let Some(job) = holder.take() {
                            // Call the custom handler
                            match handler.handle_backpressure(job) {
                                Ok(Some(retry_job)) => {
                                    // Handler wants to retry with (possibly modified) job
                                    // Use blocking send for retry
                                    queue.send(retry_job).map_err(|e| match e {
                                        QueueError::Closed(_) => ThreadError::shutting_down(0),
                                        _ => ThreadError::QueueSendError,
                                    })
                                }
                                Ok(None) => {
                                    // Handler chose to drop the job
                                    Ok(())
                                }
                                Err(e) => Err(e),
                            }
                        } else {
                            // Job holder was empty (shouldn't happen in normal operation)
                            Err(ThreadError::QueueSendError)
                        }
                    }
                    Err(QueueError::Closed(_)) => Err(ThreadError::shutting_down(0)),
                    Err(_) => Err(ThreadError::QueueSendError),
                }
            }
        };

        if result.is_ok() {
            self.total_jobs_submitted.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Submit a job with a specific priority.
    ///
    /// Jobs with higher priority are processed before jobs with lower priority.
    /// Within the same priority level, jobs are processed in FIFO order.
    ///
    /// If the pool was not configured with `enable_priority(true)` or a custom
    /// priority queue, the priority is ignored and the job is submitted normally.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::prelude::*;
    ///
    /// let config = ThreadPoolConfig::new(4).enable_priority(true);
    /// let pool = ThreadPool::with_config(config)?;
    /// pool.start()?;
    ///
    /// // Critical job will be processed first
    /// pool.submit_with_priority(my_critical_job, Priority::Critical)?;
    /// pool.submit_with_priority(my_background_job, Priority::Low)?;
    /// ```
    #[cfg(feature = "priority-scheduling")]
    pub fn submit_with_priority<J: Job + 'static>(&self, job: J, priority: Priority) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        let queue_guard = self.queue.read();
        let queue = queue_guard
            .as_ref()
            .ok_or_else(|| ThreadError::not_running(&self.config.thread_name_prefix))?;

        queue
            .send_with_priority(Box::new(job), priority)
            .map_err(|e| match e {
                QueueError::Closed(_) => ThreadError::shutting_down(0),
                QueueError::Full(_) => {
                    ThreadError::queue_full(queue.len(), self.config.max_queue_size)
                }
                _ => ThreadError::QueueSendError,
            })?;

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

    /// Execute a closure with a specific priority.
    ///
    /// Jobs with higher priority are processed before jobs with lower priority.
    /// Within the same priority level, jobs are processed in FIFO order.
    ///
    /// If the pool was not configured with `enable_priority(true)` or a custom
    /// priority queue, the priority is ignored and the job is submitted normally.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_thread_system::prelude::*;
    ///
    /// let config = ThreadPoolConfig::new(4).enable_priority(true);
    /// let pool = ThreadPool::with_config(config)?;
    /// pool.start()?;
    ///
    /// // Critical task will be processed first
    /// pool.execute_with_priority(|| {
    ///     println!("Critical work!");
    ///     Ok(())
    /// }, Priority::Critical)?;
    ///
    /// // Low priority background task
    /// pool.execute_with_priority(|| {
    ///     println!("Background work");
    ///     Ok(())
    /// }, Priority::Low)?;
    /// ```
    #[cfg(feature = "priority-scheduling")]
    pub fn execute_with_priority<F>(&self, f: F, priority: Priority) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.submit_with_priority(ClosureJob::new(f), priority)
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
    /// let pool = ThreadPool::with_threads(2)?;
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

        let job = CancellableJob::new(job_id, token.clone(), f);

        self.submit(job)?;

        Ok(handle)
    }

    /// Attempts to submit a job without blocking.
    ///
    /// Returns immediately if the queue is full instead of waiting for space.
    /// For unbounded queues, this behaves the same as `submit()`.
    ///
    /// # Errors
    ///
    /// - `ThreadError::NotRunning` - Pool is not running
    /// - `ThreadError::QueueFull` - Queue is at capacity (bounded queue only)
    /// - `ThreadError::ShuttingDown` - Pool is shutting down
    ///
    /// # Example
    ///
    /// ```
    /// use rust_thread_system::prelude::*;
    ///
    /// # fn main() -> Result<()> {
    /// let config = ThreadPoolConfig::new(2).with_max_queue_size(10);
    /// let pool = ThreadPool::with_config(config)?;
    /// pool.start()?;
    ///
    /// match pool.try_execute(|| {
    ///     println!("Job executed");
    ///     Ok(())
    /// }) {
    ///     Ok(()) => println!("Job submitted"),
    ///     Err(ThreadError::QueueFull { .. }) => println!("Queue is full, try later"),
    ///     Err(e) => println!("Error: {}", e),
    /// }
    /// # pool.shutdown()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_submit<J: Job + 'static>(&self, job: J) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        let queue_guard = self.queue.read();
        let queue = queue_guard
            .as_ref()
            .ok_or_else(|| ThreadError::not_running(&self.config.thread_name_prefix))?;

        queue.try_send(Box::new(job)).map_err(|e| match e {
            QueueError::Closed(_) => ThreadError::shutting_down(0),
            QueueError::Full(_) => ThreadError::queue_full(queue.len(), self.config.max_queue_size),
            _ => ThreadError::QueueSendError,
        })?;

        self.total_jobs_submitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Attempts to execute a closure without blocking.
    ///
    /// Returns immediately if the queue is full instead of waiting for space.
    /// For unbounded queues, this behaves the same as `execute()`.
    ///
    /// # Errors
    ///
    /// - `ThreadError::NotRunning` - Pool is not running
    /// - `ThreadError::QueueFull` - Queue is at capacity (bounded queue only)
    /// - `ThreadError::ShuttingDown` - Pool is shutting down
    pub fn try_execute<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.try_submit(ClosureJob::new(f))
    }

    /// Submits a job with a timeout.
    ///
    /// Waits up to the specified duration for space in the queue.
    /// For unbounded queues, this behaves the same as `submit()`.
    ///
    /// # Errors
    ///
    /// - `ThreadError::NotRunning` - Pool is not running
    /// - `ThreadError::SubmissionTimeout` - Timed out waiting for queue space
    /// - `ThreadError::ShuttingDown` - Pool is shutting down
    ///
    /// # Example
    ///
    /// ```
    /// use rust_thread_system::prelude::*;
    /// use std::time::Duration;
    ///
    /// # fn main() -> Result<()> {
    /// let config = ThreadPoolConfig::new(2).with_max_queue_size(10);
    /// let pool = ThreadPool::with_config(config)?;
    /// pool.start()?;
    ///
    /// match pool.execute_timeout(|| {
    ///     println!("Job executed");
    ///     Ok(())
    /// }, Duration::from_millis(100)) {
    ///     Ok(()) => println!("Job submitted"),
    ///     Err(ThreadError::SubmissionTimeout { .. }) => println!("Submission timed out"),
    ///     Err(e) => println!("Error: {}", e),
    /// }
    /// # pool.shutdown()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn submit_timeout<J: Job + 'static>(&self, job: J, timeout: Duration) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Err(ThreadError::not_running(&self.config.thread_name_prefix));
        }

        let queue_guard = self.queue.read();
        let queue = queue_guard
            .as_ref()
            .ok_or_else(|| ThreadError::not_running(&self.config.thread_name_prefix))?;

        queue
            .send_timeout(Box::new(job), timeout)
            .map_err(|e| match e {
                QueueError::Closed(_) => ThreadError::shutting_down(0),
                QueueError::Timeout(_) => {
                    ThreadError::submission_timeout(timeout.as_millis() as u64)
                }
                QueueError::Full(_) => {
                    ThreadError::queue_full(queue.len(), self.config.max_queue_size)
                }
                _ => ThreadError::QueueSendError,
            })?;

        self.total_jobs_submitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Executes a closure with a timeout.
    ///
    /// Waits up to the specified duration for space in the queue.
    /// For unbounded queues, this behaves the same as `execute()`.
    ///
    /// # Errors
    ///
    /// - `ThreadError::NotRunning` - Pool is not running
    /// - `ThreadError::SubmissionTimeout` - Timed out waiting for queue space
    /// - `ThreadError::ShuttingDown` - Pool is shutting down
    pub fn execute_timeout<F>(&self, f: F, timeout: Duration) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        self.submit_timeout(ClosureJob::new(f), timeout)
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
        self.queue
            .read()
            .as_ref()
            .map(|q| q.len() as u64)
            .unwrap_or(0)
    }

    /// Get statistics for all workers
    pub fn get_stats(&self) -> Vec<Arc<WorkerStats>> {
        self.workers.read().iter().map(|w| w.stats()).collect()
    }

    /// Get total jobs processed across all workers
    pub fn total_jobs_processed(&self) -> u64 {
        let workers = self.workers.read();
        workers.iter().map(|w| w.stats().get_jobs_processed()).sum()
    }

    /// Get total jobs failed across all workers
    pub fn total_jobs_failed(&self) -> u64 {
        let workers = self.workers.read();
        workers.iter().map(|w| w.stats().get_jobs_failed()).sum()
    }

    /// Get total jobs panicked across all workers
    pub fn total_jobs_panicked(&self) -> u64 {
        let workers = self.workers.read();
        workers.iter().map(|w| w.stats().get_jobs_panicked()).sum()
    }

    /// Shutdown the thread pool and wait for all workers to finish
    ///
    /// # Graceful Shutdown
    ///
    /// 1. Stops accepting new jobs (sets running = false)
    /// 2. Closes the queue
    /// 3. Waits for all workers to drain queued jobs and exit
    ///
    /// This ensures all queued jobs are processed before shutdown completes.
    ///
    /// # Thread Safety
    ///
    /// This method uses interior mutability and can be called from `&self`.
    /// Multiple concurrent calls are safe - only the first will perform the
    /// shutdown, others will return immediately.
    pub fn shutdown(&self) -> Result<()> {
        if !self.running.load(Ordering::Acquire) {
            return Ok(());
        }

        // Mark as not running first to prevent new job submissions
        self.running.store(false, Ordering::Release);

        // Close the queue to signal workers to shutdown
        if let Some(queue) = self.queue.read().as_ref() {
            queue.close();
        }

        // Wait for all workers to finish draining the queue
        let workers = std::mem::take(&mut *self.workers.write());
        for worker in workers {
            worker.join()?;
        }

        // Clear the queue reference
        *self.queue.write() = None;

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
        let pool = ThreadPool::new().expect("Failed to create thread pool");
        assert!(!pool.is_running());

        pool.start().expect("Failed to start pool");
        assert!(pool.is_running());
        assert_eq!(pool.num_threads(), num_cpus::get());

        pool.shutdown().expect("Failed to shutdown pool");
        assert!(!pool.is_running());
    }

    #[test]
    fn test_thread_pool_with_threads() {
        let pool = ThreadPool::with_threads(4).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");
        assert_eq!(pool.num_threads(), 4);
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_job_execution() {
        let pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
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
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
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
        let pool = ThreadPool::with_threads(4).expect("Failed to create thread pool");
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

        let pool = ThreadPool::with_threads(4).expect("Failed to create thread pool");
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

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_shutdown_waits_for_jobs() {
        let pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
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
        let pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
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
        let pool = ThreadPool::with_threads(2).expect("Failed to create thread pool");
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

    #[test]
    fn test_try_execute_returns_immediately_when_queue_full() {
        // Use a very small queue to easily fill it
        let config = ThreadPoolConfig::new(1).with_max_queue_size(2);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Use a channel to signal when the first job starts executing
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        // Submit a job that blocks the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            // Wait until test signals to complete
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit first job");

        // Wait for the worker to pick up the first job
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("First job should start within 5 seconds");

        // Now the worker is blocked, fill the queue (size 2)
        pool.try_execute(|| Ok(()))
            .expect("Failed to submit second job");

        pool.try_execute(|| Ok(()))
            .expect("Failed to submit third job");

        // Queue is now full, this should fail immediately
        let result = pool.try_execute(|| Ok(()));
        assert!(
            matches!(result, Err(ThreadError::QueueFull { .. })),
            "Expected QueueFull error, got: {:?}",
            result
        );

        // Release the blocking job to allow shutdown
        let _ = done_tx.send(());
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_try_execute_succeeds_with_unbounded_queue() {
        let config = ThreadPoolConfig::new(2).with_max_queue_size(0); // unbounded
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit many jobs - should all succeed
        for _ in 0..100 {
            let counter_clone = Arc::clone(&counter);
            pool.try_execute(move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .expect("try_execute should not fail with unbounded queue");
        }

        thread::sleep(Duration::from_millis(200));
        assert_eq!(counter.load(Ordering::Relaxed), 100);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_try_execute_when_not_running() {
        let pool = ThreadPool::new().expect("Failed to create thread pool");
        let result = pool.try_execute(|| Ok(()));
        assert!(matches!(result, Err(ThreadError::NotRunning { .. })));
    }

    #[test]
    fn test_execute_timeout_succeeds_within_timeout() {
        let config = ThreadPoolConfig::new(2).with_max_queue_size(10);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let result = pool.execute_timeout(
            move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
            Duration::from_secs(1),
        );

        assert!(result.is_ok());
        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_execute_timeout_times_out_when_queue_full() {
        // Use a very small queue to easily fill it
        let config = ThreadPoolConfig::new(1).with_max_queue_size(1);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Use a channel to signal when the first job starts executing
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        // Submit a job that blocks the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            // Wait until test signals to complete
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit blocking job");

        // Wait for the worker to pick up the first job
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("First job should start within 5 seconds");

        // Fill the queue (size 1)
        pool.execute(|| Ok(())).expect("Failed to fill queue");

        // This should timeout
        let start = std::time::Instant::now();
        let result = pool.execute_timeout(|| Ok(()), Duration::from_millis(50));
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(ThreadError::SubmissionTimeout { .. })),
            "Expected SubmissionTimeout error, got: {:?}",
            result
        );
        // Verify it actually waited approximately the timeout duration
        assert!(
            elapsed >= Duration::from_millis(40),
            "Should have waited at least 40ms, but only waited {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "Should not have waited more than 200ms, but waited {:?}",
            elapsed
        );

        // Release the blocking job to allow shutdown
        let _ = done_tx.send(());
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_execute_timeout_when_not_running() {
        let pool = ThreadPool::new().expect("Failed to create thread pool");
        let result = pool.execute_timeout(|| Ok(()), Duration::from_millis(100));
        assert!(matches!(result, Err(ThreadError::NotRunning { .. })));
    }

    #[test]
    fn test_submit_timeout_with_unbounded_queue() {
        let config = ThreadPoolConfig::new(2).with_max_queue_size(0); // unbounded
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit many jobs with timeout - should all succeed immediately
        for _ in 0..50 {
            let counter_clone = Arc::clone(&counter);
            pool.execute_timeout(
                move || {
                    counter_clone.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
                Duration::from_millis(10),
            )
            .expect("submit_timeout should not fail with unbounded queue");
        }

        thread::sleep(Duration::from_millis(200));
        assert_eq!(counter.load(Ordering::Relaxed), 50);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_poll_interval_configuration() {
        let config = ThreadPoolConfig::new(2).with_poll_interval(Duration::from_millis(50));
        assert_eq!(config.poll_interval, Duration::from_millis(50));

        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        pool.execute(move || {
            counter_clone.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .expect("Failed to submit job");

        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_poll_interval_default() {
        let config = ThreadPoolConfig::default();
        assert_eq!(config.poll_interval, Duration::from_millis(100));
    }

    #[test]
    #[should_panic(expected = "poll interval must be non-zero")]
    fn test_poll_interval_zero_panics() {
        let _ = ThreadPoolConfig::new(2).with_poll_interval(Duration::ZERO);
    }

    #[test]
    fn test_short_poll_interval_faster_shutdown() {
        // Test that shorter poll interval results in faster shutdown detection
        let config = ThreadPoolConfig::new(1).with_poll_interval(Duration::from_millis(10));
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let start = std::time::Instant::now();
        pool.shutdown().expect("Failed to shutdown pool");
        let elapsed = start.elapsed();

        // With 10ms poll interval, shutdown should complete quickly
        assert!(
            elapsed < Duration::from_millis(100),
            "Shutdown took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_custom_queue() {
        // Test using a custom queue via config
        let custom_queue = Arc::new(ChannelQueue::unbounded());
        let config =
            ThreadPoolConfig::new(2).with_queue(Arc::clone(&custom_queue) as Arc<dyn JobQueue>);

        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        pool.execute(move || {
            counter_clone.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .expect("Failed to submit job");

        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[cfg(feature = "priority-scheduling")]
    #[test]
    fn test_enable_priority_config() {
        let config = ThreadPoolConfig::new(2).enable_priority(true);
        assert!(config.enable_priority);

        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        pool.execute(move || {
            counter_clone.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .expect("Failed to submit job");

        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[cfg(feature = "priority-scheduling")]
    #[test]
    fn test_execute_with_priority() {
        use crate::core::Priority;

        let config = ThreadPoolConfig::new(2).enable_priority(true);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit jobs with different priorities
        for priority in [
            Priority::Low,
            Priority::Normal,
            Priority::High,
            Priority::Critical,
        ] {
            let counter_clone = Arc::clone(&counter);
            pool.execute_with_priority(
                move || {
                    counter_clone.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
                priority,
            )
            .expect("Failed to submit job");
        }

        thread::sleep(Duration::from_millis(200));
        assert_eq!(counter.load(Ordering::Relaxed), 4);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[cfg(feature = "priority-scheduling")]
    #[test]
    fn test_priority_ordering() {
        use crate::core::Priority;
        use std::sync::Mutex;

        // Use single thread to enforce sequential processing
        let config = ThreadPoolConfig::new(1).enable_priority(true);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");

        // Channel to signal when first job starts
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        pool.start().expect("Failed to start pool");

        // Submit a blocking job first to hold up the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit blocking job");

        // Wait for the blocking job to start
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("Blocking job should start");

        // Now submit jobs with different priorities - they'll queue up
        let order = Arc::new(Mutex::new(Vec::new()));

        for (idx, priority) in [
            (0, Priority::Low),
            (1, Priority::High),
            (2, Priority::Normal),
            (3, Priority::Critical),
        ] {
            let order_clone = Arc::clone(&order);
            pool.execute_with_priority(
                move || {
                    order_clone.lock().unwrap().push((idx, priority));
                    Ok(())
                },
                priority,
            )
            .expect("Failed to submit job");
        }

        // Release the blocking job
        let _ = done_tx.send(());

        // Wait for all jobs to complete
        thread::sleep(Duration::from_millis(500));

        let execution_order = order.lock().unwrap();
        // Expected order: Critical (3), High (1), Normal (2), Low (0)
        assert_eq!(execution_order.len(), 4);
        assert_eq!(execution_order[0].1, Priority::Critical);
        assert_eq!(execution_order[1].1, Priority::High);
        assert_eq!(execution_order[2].1, Priority::Normal);
        assert_eq!(execution_order[3].1, Priority::Low);

        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[cfg(feature = "priority-scheduling")]
    #[test]
    fn test_submit_with_priority_when_not_running() {
        use crate::core::Priority;

        let config = ThreadPoolConfig::new(2).enable_priority(true);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");

        let result = pool.execute_with_priority(|| Ok(()), Priority::High);
        assert!(matches!(result, Err(ThreadError::NotRunning { .. })));
    }

    // Backpressure strategy tests

    #[test]
    fn test_backpressure_block_default() {
        // Default strategy is Block
        let config = ThreadPoolConfig::new(2).with_max_queue_size(10);
        assert!(matches!(
            config.backpressure_strategy,
            BackpressureStrategy::Block
        ));
    }

    #[test]
    fn test_backpressure_reject_immediately() {
        // Use a very small queue to easily fill it
        let config = ThreadPoolConfig::new(1)
            .with_max_queue_size(1)
            .reject_when_full();
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Use a channel to signal when the first job starts executing
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        // Submit a job that blocks the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit first job");

        // Wait for the worker to pick up the first job
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("First job should start within 5 seconds");

        // Fill the queue (size 1)
        pool.execute(|| Ok(())).expect("Failed to fill queue");

        // This should fail immediately with QueueFull
        let result = pool.execute(|| Ok(()));
        assert!(
            matches!(result, Err(ThreadError::QueueFull { .. })),
            "Expected QueueFull error, got: {:?}",
            result
        );

        // Release the blocking job
        let _ = done_tx.send(());
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_backpressure_block_with_timeout() {
        let config = ThreadPoolConfig::new(1)
            .with_max_queue_size(1)
            .block_with_timeout(Duration::from_millis(50));
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Use a channel to signal when the first job starts executing
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        // Submit a job that blocks the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit first job");

        // Wait for the worker to pick up the first job
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("First job should start within 5 seconds");

        // Fill the queue (size 1)
        pool.execute(|| Ok(())).expect("Failed to fill queue");

        // This should timeout
        let start = std::time::Instant::now();
        let result = pool.execute(|| Ok(()));
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(ThreadError::SubmissionTimeout { .. })),
            "Expected SubmissionTimeout error, got: {:?}",
            result
        );
        assert!(
            elapsed >= Duration::from_millis(40),
            "Should have waited at least 40ms, but only waited {:?}",
            elapsed
        );

        // Release the blocking job
        let _ = done_tx.send(());
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_backpressure_drop_newest() {
        let config = ThreadPoolConfig::new(1)
            .with_max_queue_size(1)
            .with_backpressure_strategy(BackpressureStrategy::DropNewest);
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Use a channel to signal when the first job starts executing
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        // Submit a job that blocks the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit first job");

        // Wait for the worker to pick up the first job
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("First job should start within 5 seconds");

        // Fill the queue (size 1)
        pool.execute(|| Ok(())).expect("Failed to fill queue");

        // This should succeed (job is silently dropped)
        let result = pool.execute(|| Ok(()));
        assert!(result.is_ok(), "DropNewest should not return error");

        // Release the blocking job
        let _ = done_tx.send(());
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_backpressure_custom_handler() {
        use crate::queue::BackpressureHandler;
        use std::sync::atomic::AtomicBool;

        struct TestHandler {
            was_called: Arc<AtomicBool>,
        }

        impl BackpressureHandler for TestHandler {
            fn handle_backpressure(
                &self,
                _job: crate::core::BoxedJob,
            ) -> crate::core::Result<Option<crate::core::BoxedJob>> {
                self.was_called.store(true, Ordering::SeqCst);
                Ok(None) // Drop the job
            }
        }

        let was_called = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(TestHandler {
            was_called: Arc::clone(&was_called),
        });

        let config = ThreadPoolConfig::new(1)
            .with_max_queue_size(1)
            .with_backpressure_strategy(BackpressureStrategy::Custom(handler));
        let pool = ThreadPool::with_config(config).expect("Failed to create thread pool");
        pool.start().expect("Failed to start pool");

        // Use a channel to signal when the first job starts executing
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        // Submit a job that blocks the worker
        pool.execute(move || {
            started_tx.send(()).unwrap();
            let _ = done_rx.recv();
            Ok(())
        })
        .expect("Failed to submit first job");

        // Wait for the worker to pick up the first job
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("First job should start within 5 seconds");

        // Fill the queue (size 1)
        pool.execute(|| Ok(())).expect("Failed to fill queue");

        // This should trigger the custom handler
        let result = pool.execute(|| Ok(()));
        assert!(result.is_ok(), "Custom handler returned Ok(None)");
        assert!(
            was_called.load(Ordering::SeqCst),
            "Custom handler should have been called"
        );

        // Release the blocking job
        let _ = done_tx.send(());
        pool.shutdown().expect("Failed to shutdown pool");
    }

    #[test]
    fn test_backpressure_config_convenience_methods() {
        // Test reject_when_full
        let config = ThreadPoolConfig::new(2).reject_when_full();
        assert!(matches!(
            config.backpressure_strategy,
            BackpressureStrategy::RejectImmediately
        ));

        // Test block_with_timeout
        let timeout = Duration::from_secs(5);
        let config = ThreadPoolConfig::new(2).block_with_timeout(timeout);
        match config.backpressure_strategy {
            BackpressureStrategy::BlockWithTimeout(t) => {
                assert_eq!(t, timeout);
            }
            _ => panic!("Expected BlockWithTimeout"),
        }

        // Test with_backpressure_strategy
        let config = ThreadPoolConfig::new(2)
            .with_backpressure_strategy(BackpressureStrategy::DropNewest);
        assert!(matches!(
            config.backpressure_strategy,
            BackpressureStrategy::DropNewest
        ));
    }
}
