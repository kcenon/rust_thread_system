//! Adaptive queue that automatically switches between mutex-based and lock-free strategies.
//!
//! This module provides an adaptive queue that monitors contention levels and automatically
//! switches between mutex-based and lock-free implementations based on runtime conditions.
//!
//! # Overview
//!
//! Under low contention, mutex-based queues have lower overhead. Under high contention,
//! lock-free queues avoid lock contention. The adaptive queue automatically selects the
//! optimal strategy based on observed contention patterns.
//!
//! # Example
//!
//! ```rust
//! use rust_thread_system::queue::{AdaptiveQueue, AdaptiveQueueConfig, QueueStrategy, JobQueue};
//! use rust_thread_system::core::ClosureJob;
//! use std::time::Duration;
//!
//! let config = AdaptiveQueueConfig {
//!     high_contention_threshold: 0.7,
//!     low_contention_threshold: 0.3,
//!     measurement_window: 100,
//!     switch_cooldown: Duration::from_millis(100),
//!     initial_strategy: QueueStrategy::Mutex,
//! };
//!
//! let queue = AdaptiveQueue::new(config);
//!
//! // Under low load: uses efficient mutex queue
//! // Under high load: automatically switches to lock-free
//! // When load decreases: switches back to mutex
//! ```

use super::{BoxedJobHolder, JobQueue, QueueCapabilities, QueueError, QueueResult};
use crate::core::BoxedJob;
use crossbeam::queue::SegQueue;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Queue strategy for the adaptive queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueStrategy {
    /// Mutex-based queue (lower overhead, optimal for low contention)
    Mutex,
    /// Lock-free queue (no lock contention, optimal for high contention)
    LockFree,
}

impl From<u8> for QueueStrategy {
    fn from(value: u8) -> Self {
        match value {
            0 => QueueStrategy::Mutex,
            _ => QueueStrategy::LockFree,
        }
    }
}

impl From<QueueStrategy> for u8 {
    fn from(value: QueueStrategy) -> Self {
        match value {
            QueueStrategy::Mutex => 0,
            QueueStrategy::LockFree => 1,
        }
    }
}

/// Configuration for the adaptive queue.
#[derive(Clone, Debug)]
pub struct AdaptiveQueueConfig {
    /// Threshold to switch to lock-free (contention ratio 0.0-1.0).
    /// When contention exceeds this threshold, switches to lock-free strategy.
    pub high_contention_threshold: f64,

    /// Threshold to switch back to mutex (contention ratio 0.0-1.0).
    /// When contention drops below this threshold, switches back to mutex strategy.
    pub low_contention_threshold: f64,

    /// Number of operations in the measurement window for calculating contention.
    pub measurement_window: usize,

    /// Minimum time between strategy switches to avoid thrashing.
    pub switch_cooldown: Duration,

    /// Initial queue strategy.
    pub initial_strategy: QueueStrategy,
}

impl Default for AdaptiveQueueConfig {
    fn default() -> Self {
        Self {
            high_contention_threshold: 0.7,
            low_contention_threshold: 0.3,
            measurement_window: 1000,
            switch_cooldown: Duration::from_millis(100),
            initial_strategy: QueueStrategy::Mutex,
        }
    }
}

/// Statistics for the adaptive queue.
#[derive(Clone, Debug, Default)]
pub struct AdaptiveQueueStats {
    /// Current active strategy.
    pub current_strategy: Option<QueueStrategy>,

    /// Number of strategy switches that have occurred.
    pub switch_count: u64,

    /// Current contention ratio (0.0-1.0).
    pub contention_ratio: f64,

    /// Total operations on the mutex queue.
    pub mutex_ops: u64,

    /// Total operations on the lock-free queue.
    pub lockfree_ops: u64,

    /// Total contended operations (waited for lock).
    pub contended_ops: u64,

    /// Total operations measured.
    pub total_ops: u64,
}

/// Tracks contention levels for adaptive strategy selection.
struct ContentionTracker {
    /// Rolling window of operation wait times.
    latencies: Mutex<VecDeque<Duration>>,

    /// Count of contended operations (waited for lock).
    contended_ops: AtomicU64,

    /// Total operations in window.
    total_ops: AtomicU64,

    /// Operations on mutex queue.
    mutex_ops: AtomicU64,

    /// Operations on lock-free queue.
    lockfree_ops: AtomicU64,

    /// Last strategy switch time (nanos since UNIX epoch).
    last_switch: AtomicU64,

    /// Number of strategy switches.
    switch_count: AtomicU64,

    /// Configuration reference.
    measurement_window: usize,
}

impl ContentionTracker {
    fn new(config: &AdaptiveQueueConfig) -> Self {
        Self {
            latencies: Mutex::new(VecDeque::with_capacity(config.measurement_window)),
            contended_ops: AtomicU64::new(0),
            total_ops: AtomicU64::new(0),
            mutex_ops: AtomicU64::new(0),
            lockfree_ops: AtomicU64::new(0),
            last_switch: AtomicU64::new(0),
            switch_count: AtomicU64::new(0),
            measurement_window: config.measurement_window,
        }
    }

    /// Records an operation and returns the current contention ratio.
    fn record_operation(&self, waited: bool, latency: Duration, strategy: QueueStrategy) -> f64 {
        // Update operation counts
        self.total_ops.fetch_add(1, Ordering::Relaxed);

        if waited {
            self.contended_ops.fetch_add(1, Ordering::Relaxed);
        }

        match strategy {
            QueueStrategy::Mutex => {
                self.mutex_ops.fetch_add(1, Ordering::Relaxed);
            }
            QueueStrategy::LockFree => {
                self.lockfree_ops.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Update latency window
        {
            let mut latencies = self.latencies.lock();
            latencies.push_back(latency);
            while latencies.len() > self.measurement_window {
                latencies.pop_front();
            }
        }

        // Calculate rolling contention ratio
        self.contention_ratio()
    }

    /// Returns the current contention ratio.
    fn contention_ratio(&self) -> f64 {
        let contended = self.contended_ops.load(Ordering::Relaxed);
        let total = self.total_ops.load(Ordering::Relaxed);

        if total == 0 {
            0.0
        } else {
            contended as f64 / total as f64
        }
    }

    /// Checks if a strategy switch should occur.
    fn should_switch(
        &self,
        current: QueueStrategy,
        ratio: f64,
        config: &AdaptiveQueueConfig,
    ) -> Option<QueueStrategy> {
        // Check cooldown
        let now_nanos = Instant::now().elapsed().as_nanos() as u64;
        let last = self.last_switch.load(Ordering::Relaxed);
        let cooldown_nanos = config.switch_cooldown.as_nanos() as u64;

        if last > 0 && now_nanos.saturating_sub(last) < cooldown_nanos {
            return None;
        }

        match current {
            QueueStrategy::Mutex if ratio > config.high_contention_threshold => {
                Some(QueueStrategy::LockFree)
            }
            QueueStrategy::LockFree if ratio < config.low_contention_threshold => {
                Some(QueueStrategy::Mutex)
            }
            _ => None,
        }
    }

    /// Records a strategy switch.
    fn record_switch(&self) {
        let now_nanos = Instant::now().elapsed().as_nanos() as u64;
        self.last_switch.store(now_nanos, Ordering::Release);
        self.switch_count.fetch_add(1, Ordering::Relaxed);
        // Reset counters for fresh measurement
        self.contended_ops.store(0, Ordering::Relaxed);
        self.total_ops.store(0, Ordering::Relaxed);
    }

    /// Returns current statistics.
    fn stats(&self, current_strategy: QueueStrategy) -> AdaptiveQueueStats {
        AdaptiveQueueStats {
            current_strategy: Some(current_strategy),
            switch_count: self.switch_count.load(Ordering::Relaxed),
            contention_ratio: self.contention_ratio(),
            mutex_ops: self.mutex_ops.load(Ordering::Relaxed),
            lockfree_ops: self.lockfree_ops.load(Ordering::Relaxed),
            contended_ops: self.contended_ops.load(Ordering::Relaxed),
            total_ops: self.total_ops.load(Ordering::Relaxed),
        }
    }
}

/// Mutex-based queue for low contention scenarios.
struct MutexQueue {
    queue: Mutex<VecDeque<BoxedJob>>,
    closed: AtomicBool,
    len: AtomicU64,
}

impl MutexQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            closed: AtomicBool::new(false),
            len: AtomicU64::new(0),
        }
    }

    fn send(&self, job: BoxedJob) -> (QueueResult<()>, bool) {
        if self.closed.load(Ordering::SeqCst) {
            return (Err(QueueError::Closed(BoxedJobHolder::new(job))), false);
        }

        // Try to acquire lock - check if we would block
        let start = Instant::now();
        let mut queue = self.queue.lock();
        let waited = start.elapsed() > Duration::from_micros(10);

        queue.push_back(job);
        self.len.fetch_add(1, Ordering::Relaxed);

        (Ok(()), waited)
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        let mut queue = self.queue.lock();
        match queue.pop_front() {
            Some(job) => {
                self.len.fetch_sub(1, Ordering::Relaxed);
                Ok(job)
            }
            None => {
                if self.closed.load(Ordering::SeqCst) {
                    Err(QueueError::Disconnected)
                } else {
                    Err(QueueError::Empty)
                }
            }
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed) as usize
    }

    fn drain(&self) -> Vec<BoxedJob> {
        let mut queue = self.queue.lock();
        let jobs: Vec<BoxedJob> = queue.drain(..).collect();
        self.len.store(0, Ordering::Relaxed);
        jobs
    }

    fn extend(&self, jobs: Vec<BoxedJob>) {
        let mut queue = self.queue.lock();
        let count = jobs.len();
        queue.extend(jobs);
        self.len.fetch_add(count as u64, Ordering::Relaxed);
    }
}

/// Lock-free queue for high contention scenarios.
struct LockFreeQueue {
    queue: SegQueue<BoxedJob>,
    closed: AtomicBool,
    len: AtomicU64,
}

impl LockFreeQueue {
    fn new() -> Self {
        Self {
            queue: SegQueue::new(),
            closed: AtomicBool::new(false),
            len: AtomicU64::new(0),
        }
    }

    fn send(&self, job: BoxedJob) -> QueueResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(QueueError::Closed(BoxedJobHolder::new(job)));
        }
        self.queue.push(job);
        self.len.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        match self.queue.pop() {
            Some(job) => {
                self.len.fetch_sub(1, Ordering::Relaxed);
                Ok(job)
            }
            None => {
                if self.closed.load(Ordering::SeqCst) {
                    Err(QueueError::Disconnected)
                } else {
                    Err(QueueError::Empty)
                }
            }
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed) as usize
    }

    fn drain(&self) -> Vec<BoxedJob> {
        let mut jobs = Vec::new();
        while let Some(job) = self.queue.pop() {
            jobs.push(job);
        }
        self.len.store(0, Ordering::Relaxed);
        jobs
    }

    fn extend(&self, jobs: Vec<BoxedJob>) {
        let count = jobs.len();
        for job in jobs {
            self.queue.push(job);
        }
        self.len.fetch_add(count as u64, Ordering::Relaxed);
    }
}

/// An adaptive queue that automatically switches between mutex-based and lock-free strategies.
///
/// The queue monitors contention levels and switches strategies to optimize performance:
/// - Under low contention: uses mutex-based queue (lower overhead)
/// - Under high contention: uses lock-free queue (no lock contention)
///
/// # Thread Safety
///
/// This queue is thread-safe and can be safely shared across multiple threads.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::queue::{AdaptiveQueue, AdaptiveQueueConfig, QueueStrategy, JobQueue};
/// use rust_thread_system::core::ClosureJob;
///
/// let config = AdaptiveQueueConfig::default();
/// let queue = AdaptiveQueue::new(config);
///
/// let job = Box::new(ClosureJob::new(|| Ok(())));
/// queue.send(job).unwrap();
/// let received = queue.try_recv().unwrap();
/// ```
pub struct AdaptiveQueue {
    /// Current active strategy.
    strategy: AtomicU8,

    /// Mutex-based queue (low contention).
    mutex_queue: MutexQueue,

    /// Lock-free queue (high contention).
    lockfree_queue: LockFreeQueue,

    /// Contention detector.
    contention_tracker: ContentionTracker,

    /// Configuration.
    config: AdaptiveQueueConfig,

    /// Lock for strategy migration.
    migration_lock: RwLock<()>,
}

impl AdaptiveQueue {
    /// Creates a new adaptive queue with the given configuration.
    pub fn new(config: AdaptiveQueueConfig) -> Self {
        let strategy = config.initial_strategy;
        Self {
            strategy: AtomicU8::new(strategy.into()),
            mutex_queue: MutexQueue::new(),
            lockfree_queue: LockFreeQueue::new(),
            contention_tracker: ContentionTracker::new(&config),
            config,
            migration_lock: RwLock::new(()),
        }
    }

    /// Returns the current queue strategy.
    pub fn current_strategy(&self) -> QueueStrategy {
        self.strategy.load(Ordering::Acquire).into()
    }

    /// Returns statistics for the adaptive queue.
    pub fn stats(&self) -> AdaptiveQueueStats {
        self.contention_tracker.stats(self.current_strategy())
    }

    /// Attempts to switch to a new strategy if conditions are met.
    fn try_switch_strategy(&self, ratio: f64) {
        let current = self.current_strategy();

        if let Some(new_strategy) =
            self.contention_tracker
                .should_switch(current, ratio, &self.config)
        {
            // Acquire write lock for migration
            let _guard = self.migration_lock.write();

            // Double-check conditions after acquiring lock
            let current = self.current_strategy();
            if self
                .contention_tracker
                .should_switch(current, ratio, &self.config)
                .is_some()
            {
                self.switch_strategy(new_strategy);
            }
        }
    }

    /// Switches to a new strategy and migrates pending jobs.
    fn switch_strategy(&self, new_strategy: QueueStrategy) {
        let old_strategy = self.current_strategy();

        // Migrate jobs from old queue to new queue
        match (old_strategy, new_strategy) {
            (QueueStrategy::Mutex, QueueStrategy::LockFree) => {
                let jobs = self.mutex_queue.drain();
                self.lockfree_queue.extend(jobs);
            }
            (QueueStrategy::LockFree, QueueStrategy::Mutex) => {
                let jobs = self.lockfree_queue.drain();
                self.mutex_queue.extend(jobs);
            }
            _ => {}
        }

        // Update strategy
        self.strategy.store(new_strategy.into(), Ordering::Release);
        self.contention_tracker.record_switch();
    }
}

impl JobQueue for AdaptiveQueue {
    fn send(&self, job: BoxedJob) -> QueueResult<()> {
        let _guard = self.migration_lock.read();
        let strategy = self.current_strategy();
        let start = Instant::now();

        let (result, waited) = match strategy {
            QueueStrategy::Mutex => self.mutex_queue.send(job),
            QueueStrategy::LockFree => (self.lockfree_queue.send(job), false),
        };

        let latency = start.elapsed();
        let ratio = self
            .contention_tracker
            .record_operation(waited, latency, strategy);

        drop(_guard);
        self.try_switch_strategy(ratio);

        result
    }

    fn try_send(&self, job: BoxedJob) -> QueueResult<()> {
        let _guard = self.migration_lock.read();
        let strategy = self.current_strategy();

        match strategy {
            QueueStrategy::Mutex => self.mutex_queue.send(job).0,
            QueueStrategy::LockFree => self.lockfree_queue.send(job),
        }
    }

    fn send_timeout(&self, job: BoxedJob, _timeout: Duration) -> QueueResult<()> {
        // For unbounded queues, send_timeout is equivalent to send
        self.send(job)
    }

    fn recv(&self) -> QueueResult<BoxedJob> {
        // Blocking receive - try both queues with polling
        loop {
            match self.try_recv() {
                Ok(job) => return Ok(job),
                Err(QueueError::Empty) => {
                    if self.is_closed() {
                        return Err(QueueError::Disconnected);
                    }
                    std::thread::sleep(Duration::from_micros(100));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        let _guard = self.migration_lock.read();
        let strategy = self.current_strategy();

        match strategy {
            QueueStrategy::Mutex => self.mutex_queue.try_recv(),
            QueueStrategy::LockFree => self.lockfree_queue.try_recv(),
        }
    }

    fn recv_timeout(&self, timeout: Duration) -> QueueResult<BoxedJob> {
        let start = Instant::now();

        loop {
            match self.try_recv() {
                Ok(job) => return Ok(job),
                Err(QueueError::Empty) => {
                    if self.is_closed() {
                        return Err(QueueError::Disconnected);
                    }
                    if start.elapsed() >= timeout {
                        return Err(QueueError::Empty);
                    }
                    std::thread::sleep(Duration::from_micros(100));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn close(&self) {
        self.mutex_queue.close();
        self.lockfree_queue.close();
    }

    fn is_closed(&self) -> bool {
        self.mutex_queue.is_closed() || self.lockfree_queue.is_closed()
    }

    fn len(&self) -> usize {
        self.mutex_queue.len() + self.lockfree_queue.len()
    }

    fn capabilities(&self) -> QueueCapabilities {
        QueueCapabilities {
            is_bounded: false,
            capacity: None,
            is_lock_free: self.current_strategy() == QueueStrategy::LockFree,
            supports_priority: false,
            exact_size: false,
        }
    }
}

// Safety: AdaptiveQueue is Send + Sync because:
// - All atomic types are Send + Sync
// - MutexQueue uses parking_lot::Mutex which is Send + Sync
// - LockFreeQueue uses crossbeam::SegQueue which is Send + Sync
// - RwLock is Send + Sync
unsafe impl Send for AdaptiveQueue {}
unsafe impl Sync for AdaptiveQueue {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ClosureJob;
    use std::sync::Arc;
    use std::thread;

    fn create_test_job() -> BoxedJob {
        Box::new(ClosureJob::new(|| Ok(())))
    }

    #[test]
    fn test_adaptive_queue_creation() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);
        assert_eq!(queue.current_strategy(), QueueStrategy::Mutex);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_adaptive_queue_send_recv() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);

        queue.send(create_test_job()).unwrap();
        assert_eq!(queue.len(), 1);

        let job = queue.try_recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
        assert!(queue.is_empty());
    }

    #[test]
    fn test_adaptive_queue_try_recv_empty() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);

        match queue.try_recv() {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error"),
        }
    }

    #[test]
    fn test_adaptive_queue_close() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);

        assert!(!queue.is_closed());
        queue.close();
        assert!(queue.is_closed());

        match queue.send(create_test_job()) {
            Err(QueueError::Closed(_)) => {}
            _ => panic!("expected Closed error"),
        }
    }

    #[test]
    fn test_adaptive_queue_stats() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);

        for _ in 0..10 {
            queue.send(create_test_job()).unwrap();
        }

        let stats = queue.stats();
        assert_eq!(stats.current_strategy, Some(QueueStrategy::Mutex));
        assert!(stats.mutex_ops > 0);
    }

    #[test]
    fn test_adaptive_queue_lockfree_initial() {
        let config = AdaptiveQueueConfig {
            initial_strategy: QueueStrategy::LockFree,
            ..Default::default()
        };
        let queue = AdaptiveQueue::new(config);
        assert_eq!(queue.current_strategy(), QueueStrategy::LockFree);

        queue.send(create_test_job()).unwrap();
        let job = queue.try_recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");

        let stats = queue.stats();
        assert!(stats.lockfree_ops > 0);
    }

    #[test]
    fn test_adaptive_queue_concurrent() {
        let config = AdaptiveQueueConfig::default();
        let queue = Arc::new(AdaptiveQueue::new(config));
        let num_jobs = 100;

        // Spawn sender threads
        let mut handles = vec![];
        for _ in 0..4 {
            let q = Arc::clone(&queue);
            handles.push(thread::spawn(move || {
                for _ in 0..num_jobs / 4 {
                    q.send(create_test_job()).unwrap();
                }
            }));
        }

        // Wait for all sends to complete
        for h in handles {
            h.join().unwrap();
        }

        // Receive all jobs
        let mut received = 0;
        while queue.try_recv().is_ok() {
            received += 1;
        }
        assert_eq!(received, num_jobs);
    }

    #[test]
    fn test_adaptive_queue_capabilities() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);

        let caps = queue.capabilities();
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
        assert!(!caps.supports_priority);
    }

    #[test]
    fn test_adaptive_queue_recv_timeout() {
        let config = AdaptiveQueueConfig::default();
        let queue = AdaptiveQueue::new(config);

        let start = Instant::now();
        let result = queue.recv_timeout(Duration::from_millis(50));
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(50));
        match result {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error on timeout"),
        }
    }

    #[test]
    fn test_contention_tracker() {
        let config = AdaptiveQueueConfig::default();
        let tracker = ContentionTracker::new(&config);

        // Record some contended operations
        for _ in 0..10 {
            tracker.record_operation(true, Duration::from_micros(100), QueueStrategy::Mutex);
        }

        let ratio = tracker.contention_ratio();
        assert!((ratio - 1.0).abs() < 0.01); // All operations were contended

        // Record non-contended operations
        for _ in 0..10 {
            tracker.record_operation(false, Duration::from_micros(10), QueueStrategy::Mutex);
        }

        let ratio = tracker.contention_ratio();
        assert!((ratio - 0.5).abs() < 0.01); // Half were contended
    }

    #[test]
    fn test_mutex_queue_basic() {
        let queue = MutexQueue::new();

        let (result, _) = queue.send(create_test_job());
        assert!(result.is_ok());
        assert_eq!(queue.len(), 1);

        let job = queue.try_recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_lockfree_queue_basic() {
        let queue = LockFreeQueue::new();

        queue.send(create_test_job()).unwrap();
        assert_eq!(queue.len(), 1);

        let job = queue.try_recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_queue_strategy_conversion() {
        assert_eq!(QueueStrategy::from(0u8), QueueStrategy::Mutex);
        assert_eq!(QueueStrategy::from(1u8), QueueStrategy::LockFree);
        assert_eq!(QueueStrategy::from(255u8), QueueStrategy::LockFree);

        assert_eq!(u8::from(QueueStrategy::Mutex), 0);
        assert_eq!(u8::from(QueueStrategy::LockFree), 1);
    }
}
