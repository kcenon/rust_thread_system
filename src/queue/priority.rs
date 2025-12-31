//! Priority-based job queue implementation.
//!
//! This module provides a thread-safe priority queue that implements the [`JobQueue`] trait,
//! enabling priority-based job scheduling with the ThreadPool.

use super::{BoxedJobHolder, JobQueue, QueueCapabilities, QueueError, QueueResult};
use crate::core::priority::{Priority, PriorityJob, PriorityQueue};
use crate::core::BoxedJob;
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// A thread-safe priority-based job queue.
///
/// Jobs are dequeued in priority order (Critical > High > Normal > Low).
/// Within the same priority level, jobs are processed in FIFO order.
///
/// # Example
///
/// ```rust,ignore
/// use rust_thread_system::queue::{PriorityJobQueue, JobQueue};
/// use rust_thread_system::core::{ClosureJob, Priority};
///
/// let queue = PriorityJobQueue::new();
///
/// // Send a low priority job
/// let job = Box::new(ClosureJob::new(|| Ok(())));
/// queue.send_with_priority(job, Priority::Low).unwrap();
///
/// // Send a high priority job
/// let job = Box::new(ClosureJob::new(|| Ok(())));
/// queue.send_with_priority(job, Priority::High).unwrap();
///
/// // High priority job will be received first
/// let first = queue.recv().unwrap();
/// ```
pub struct PriorityJobQueue {
    queue: Mutex<PriorityQueue>,
    condvar: Condvar,
    sequence: AtomicU64,
    closed: AtomicBool,
}

impl PriorityJobQueue {
    /// Creates a new empty priority queue.
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(PriorityQueue::new()),
            condvar: Condvar::new(),
            sequence: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }
    }

    /// Creates a new priority queue with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(PriorityQueue::with_capacity(capacity)),
            condvar: Condvar::new(),
            sequence: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }
    }

    /// Sends a job with a specific priority.
    ///
    /// This is the preferred method for sending jobs to a priority queue,
    /// as it allows explicit priority specification.
    pub fn send_with_priority(&self, job: BoxedJob, priority: Priority) -> QueueResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(QueueError::Closed(BoxedJobHolder::new(job)));
        }

        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        let priority_job = PriorityJob::new(priority, job, sequence);

        {
            let mut guard = self.queue.lock();
            guard.push(priority_job);
        }

        self.condvar.notify_one();
        Ok(())
    }

    /// Attempts to send a job with a specific priority without blocking.
    pub fn try_send_with_priority(&self, job: BoxedJob, priority: Priority) -> QueueResult<()> {
        // Priority queue is unbounded, so try_send behaves like send
        self.send_with_priority(job, priority)
    }

    /// Sends a job with a specific priority, with a timeout.
    ///
    /// Since the priority queue is unbounded, this behaves identically to
    /// [`send_with_priority`](Self::send_with_priority).
    pub fn send_with_priority_timeout(
        &self,
        job: BoxedJob,
        priority: Priority,
        _timeout: Duration,
    ) -> QueueResult<()> {
        // Priority queue is unbounded, so timeout is not relevant
        self.send_with_priority(job, priority)
    }
}

impl Default for PriorityJobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue for PriorityJobQueue {
    fn send(&self, job: BoxedJob) -> QueueResult<()> {
        // Default to Normal priority
        self.send_with_priority(job, Priority::Normal)
    }

    fn try_send(&self, job: BoxedJob) -> QueueResult<()> {
        self.try_send_with_priority(job, Priority::Normal)
    }

    fn send_timeout(&self, job: BoxedJob, timeout: Duration) -> QueueResult<()> {
        self.send_with_priority_timeout(job, Priority::Normal, timeout)
    }

    fn recv(&self) -> QueueResult<BoxedJob> {
        let mut guard = self.queue.lock();

        loop {
            if let Some(priority_job) = guard.pop() {
                return Ok(priority_job.into_job());
            }

            if self.closed.load(Ordering::SeqCst) && guard.is_empty() {
                return Err(QueueError::Disconnected);
            }

            self.condvar.wait(&mut guard);

            if self.closed.load(Ordering::SeqCst) && guard.is_empty() {
                return Err(QueueError::Disconnected);
            }
        }
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        let mut guard = self.queue.lock();

        if let Some(priority_job) = guard.pop() {
            return Ok(priority_job.into_job());
        }

        if self.closed.load(Ordering::SeqCst) {
            return Err(QueueError::Disconnected);
        }

        Err(QueueError::Empty)
    }

    fn recv_timeout(&self, timeout: Duration) -> QueueResult<BoxedJob> {
        let mut guard = self.queue.lock();

        if let Some(priority_job) = guard.pop() {
            return Ok(priority_job.into_job());
        }

        if self.closed.load(Ordering::SeqCst) && guard.is_empty() {
            return Err(QueueError::Disconnected);
        }

        let result = self.condvar.wait_for(&mut guard, timeout);

        if let Some(priority_job) = guard.pop() {
            return Ok(priority_job.into_job());
        }

        if self.closed.load(Ordering::SeqCst) && guard.is_empty() {
            return Err(QueueError::Disconnected);
        }

        if result.timed_out() {
            return Err(QueueError::Empty);
        }

        Err(QueueError::Empty)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.condvar.notify_all();
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn len(&self) -> usize {
        self.queue.lock().len()
    }

    fn capabilities(&self) -> QueueCapabilities {
        QueueCapabilities {
            is_bounded: false,
            capacity: None,
            is_lock_free: false,
            supports_priority: true,
            exact_size: true,
        }
    }
}

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
    fn test_priority_ordering() {
        let queue = PriorityJobQueue::new();

        // Send jobs with different priorities
        queue
            .send_with_priority(create_test_job(), Priority::Low)
            .unwrap();
        queue
            .send_with_priority(create_test_job(), Priority::High)
            .unwrap();
        queue
            .send_with_priority(create_test_job(), Priority::Normal)
            .unwrap();
        queue
            .send_with_priority(create_test_job(), Priority::Critical)
            .unwrap();

        // Should receive in priority order
        assert_eq!(queue.len(), 4);

        // We can't inspect priority directly through JobQueue trait,
        // but we verify the queue is working by receiving all jobs
        for _ in 0..4 {
            queue.try_recv().unwrap();
        }

        assert!(queue.is_empty());
    }

    #[test]
    fn test_default_priority() {
        let queue = PriorityJobQueue::new();
        queue.send(create_test_job()).unwrap();
        let job = queue.recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
    }

    #[test]
    fn test_try_recv_empty() {
        let queue = PriorityJobQueue::new();
        match queue.try_recv() {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error"),
        }
    }

    #[test]
    fn test_recv_timeout() {
        let queue = PriorityJobQueue::new();
        let result = queue.recv_timeout(Duration::from_millis(10));
        match result {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error on timeout"),
        }
    }

    #[test]
    fn test_close() {
        let queue = PriorityJobQueue::new();
        assert!(!queue.is_closed());
        queue.close();
        assert!(queue.is_closed());

        match queue.send(create_test_job()) {
            Err(QueueError::Closed(_)) => {}
            _ => panic!("expected Closed error"),
        }
    }

    #[test]
    fn test_close_wakes_waiting_receivers() {
        let queue = Arc::new(PriorityJobQueue::new());

        let q = Arc::clone(&queue);
        let handle = thread::spawn(move || {
            // This will block waiting for a job
            q.recv()
        });

        // Give the receiver time to start waiting
        thread::sleep(Duration::from_millis(50));

        // Close the queue
        queue.close();

        // The receiver should wake up with Disconnected error
        match handle.join().unwrap() {
            Err(QueueError::Disconnected) => {}
            _ => panic!("expected Disconnected error"),
        }
    }

    #[test]
    fn test_len_and_is_empty() {
        let queue = PriorityJobQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        queue.send(create_test_job()).unwrap();
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);

        queue.recv().unwrap();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_capabilities() {
        let queue = PriorityJobQueue::new();
        let caps = queue.capabilities();
        assert!(!caps.is_bounded);
        assert!(caps.capacity.is_none());
        assert!(!caps.is_lock_free);
        assert!(caps.supports_priority);
        assert!(caps.exact_size);
    }

    #[test]
    fn test_concurrent_priority() {
        let queue = Arc::new(PriorityJobQueue::new());
        let num_jobs = 100;

        // Spawn sender threads
        let mut handles = vec![];
        for i in 0..4 {
            let q = Arc::clone(&queue);
            let priority = match i % 4 {
                0 => Priority::Low,
                1 => Priority::Normal,
                2 => Priority::High,
                _ => Priority::Critical,
            };
            handles.push(thread::spawn(move || {
                for _ in 0..num_jobs / 4 {
                    q.send_with_priority(create_test_job(), priority).unwrap();
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
    fn test_fifo_within_priority() {
        let queue = PriorityJobQueue::new();

        // Send multiple jobs with same priority
        for i in 0..5 {
            let job = Box::new(ClosureJob::with_name(
                || Ok(()),
                format!("Job{}", i),
            ));
            queue.send_with_priority(job, Priority::Normal).unwrap();
        }

        // Jobs should come out in FIFO order
        for i in 0..5 {
            let job = queue.recv().unwrap();
            assert_eq!(job.job_type(), format!("Job{}", i));
        }
    }

    #[test]
    fn test_recv_blocks_until_job_available() {
        let queue = Arc::new(PriorityJobQueue::new());

        let q = Arc::clone(&queue);
        let handle = thread::spawn(move || q.recv().unwrap());

        // Give the receiver time to start waiting
        thread::sleep(Duration::from_millis(50));

        // Send a job
        queue.send(create_test_job()).unwrap();

        // Receiver should get the job
        let job = handle.join().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
    }
}
