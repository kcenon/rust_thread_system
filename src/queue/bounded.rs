//! Bounded FIFO queue with capacity limit.

use super::{BoxedJobHolder, JobQueue, QueueCapabilities, QueueError, QueueResult};
use crate::core::BoxedJob;
use crossbeam::channel::{self, Receiver, Sender, TryRecvError, TrySendError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A bounded FIFO queue with configurable capacity.
///
/// This queue implementation provides backpressure by blocking or returning
/// errors when the queue is full, preventing memory exhaustion in high-load
/// scenarios.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::queue::{BoundedQueue, JobQueue, QueueError};
/// use rust_thread_system::core::ClosureJob;
///
/// let queue = BoundedQueue::new(2);
///
/// // Fill the queue
/// let job1 = Box::new(ClosureJob::new(|| Ok(())));
/// let job2 = Box::new(ClosureJob::new(|| Ok(())));
/// queue.send(job1).unwrap();
/// queue.send(job2).unwrap();
///
/// // Queue is now full - try_send will fail
/// let job3 = Box::new(ClosureJob::new(|| Ok(())));
/// match queue.try_send(job3) {
///     Err(QueueError::Full(_)) => println!("Queue is full"),
///     _ => panic!("expected Full error"),
/// }
/// ```
pub struct BoundedQueue {
    sender: Sender<BoxedJob>,
    receiver: Receiver<BoxedJob>,
    capacity: usize,
    closed: AtomicBool,
}

impl BoundedQueue {
    /// Creates a new bounded queue with the specified capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The maximum number of jobs the queue can hold.
    ///   Must be greater than 0.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be greater than 0");
        let (sender, receiver) = channel::bounded(capacity);
        Self {
            sender,
            receiver,
            capacity,
            closed: AtomicBool::new(false),
        }
    }

    /// Returns the maximum capacity of this queue.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl JobQueue for BoundedQueue {
    fn send(&self, job: BoxedJob) -> QueueResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(QueueError::Closed(BoxedJobHolder::new(job)));
        }
        self.sender
            .send(job)
            .map_err(|e| QueueError::Closed(BoxedJobHolder::new(e.0)))
    }

    fn try_send(&self, job: BoxedJob) -> QueueResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(QueueError::Closed(BoxedJobHolder::new(job)));
        }
        self.sender.try_send(job).map_err(|e| match e {
            TrySendError::Full(job) => QueueError::Full(BoxedJobHolder::new(job)),
            TrySendError::Disconnected(job) => QueueError::Closed(BoxedJobHolder::new(job)),
        })
    }

    fn send_timeout(&self, job: BoxedJob, timeout: Duration) -> QueueResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(QueueError::Closed(BoxedJobHolder::new(job)));
        }
        self.sender.send_timeout(job, timeout).map_err(|e| match e {
            channel::SendTimeoutError::Timeout(job) => {
                QueueError::Timeout(BoxedJobHolder::new(job))
            }
            channel::SendTimeoutError::Disconnected(job) => {
                QueueError::Closed(BoxedJobHolder::new(job))
            }
        })
    }

    fn recv(&self) -> QueueResult<BoxedJob> {
        self.receiver.recv().map_err(|_| QueueError::Disconnected)
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        self.receiver.try_recv().map_err(|e| match e {
            TryRecvError::Empty => QueueError::Empty,
            TryRecvError::Disconnected => QueueError::Disconnected,
        })
    }

    fn recv_timeout(&self, timeout: Duration) -> QueueResult<BoxedJob> {
        // Check if closed first
        if self.closed.load(Ordering::SeqCst) && self.receiver.is_empty() {
            return Err(QueueError::Disconnected);
        }

        match self.receiver.recv_timeout(timeout) {
            Ok(job) => Ok(job),
            Err(channel::RecvTimeoutError::Timeout) => {
                // On timeout, check if closed
                if self.closed.load(Ordering::SeqCst) && self.receiver.is_empty() {
                    Err(QueueError::Disconnected)
                } else {
                    Err(QueueError::Empty)
                }
            }
            Err(channel::RecvTimeoutError::Disconnected) => Err(QueueError::Disconnected),
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn len(&self) -> usize {
        self.receiver.len()
    }

    fn capabilities(&self) -> QueueCapabilities {
        QueueCapabilities {
            is_bounded: true,
            capacity: Some(self.capacity),
            is_lock_free: true,
            supports_priority: false,
            exact_size: false,
        }
    }
}

// Safety: BoundedQueue is Send + Sync because:
// - Sender and Receiver from crossbeam are Send + Sync
// - AtomicBool is Send + Sync
unsafe impl Send for BoundedQueue {}
unsafe impl Sync for BoundedQueue {}

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
    fn test_bounded_send_recv() {
        let queue = BoundedQueue::new(10);
        queue.send(create_test_job()).unwrap();
        let job = queue.recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
    }

    #[test]
    fn test_capacity() {
        let queue = BoundedQueue::new(5);
        assert_eq!(queue.capacity(), 5);
    }

    #[test]
    #[should_panic(expected = "capacity must be greater than 0")]
    fn test_zero_capacity_panics() {
        let _ = BoundedQueue::new(0);
    }

    #[test]
    fn test_try_send_full() {
        let queue = BoundedQueue::new(2);
        queue.try_send(create_test_job()).unwrap();
        queue.try_send(create_test_job()).unwrap();

        // Queue is now full
        match queue.try_send(create_test_job()) {
            Err(QueueError::Full(holder)) => {
                // Job should be recoverable
                let recovered = holder.take();
                assert!(recovered.is_some());
            }
            _ => panic!("expected Full error"),
        }
    }

    #[test]
    fn test_send_blocks_when_full() {
        let queue = Arc::new(BoundedQueue::new(1));
        queue.send(create_test_job()).unwrap();

        let q = Arc::clone(&queue);
        let handle = thread::spawn(move || {
            // This should block until the queue has space
            q.send(create_test_job()).unwrap();
        });

        // Give the sender a chance to block
        thread::sleep(Duration::from_millis(10));

        // Receive to make space
        queue.recv().unwrap();

        // Now the sender should unblock
        handle.join().unwrap();
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_send_timeout_when_full() {
        let queue = BoundedQueue::new(1);
        queue.send(create_test_job()).unwrap();

        // This should timeout
        match queue.send_timeout(create_test_job(), Duration::from_millis(10)) {
            Err(QueueError::Timeout(holder)) => {
                let recovered = holder.take();
                assert!(recovered.is_some());
            }
            _ => panic!("expected Timeout error"),
        }
    }

    #[test]
    fn test_try_recv_empty() {
        let queue = BoundedQueue::new(10);
        match queue.try_recv() {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error"),
        }
    }

    #[test]
    fn test_recv_timeout() {
        let queue = BoundedQueue::new(10);
        let result = queue.recv_timeout(Duration::from_millis(10));
        match result {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error on timeout"),
        }
    }

    #[test]
    fn test_close() {
        let queue = BoundedQueue::new(10);
        assert!(!queue.is_closed());
        queue.close();
        assert!(queue.is_closed());

        match queue.send(create_test_job()) {
            Err(QueueError::Closed(_)) => {}
            _ => panic!("expected Closed error"),
        }
    }

    #[test]
    fn test_len_and_is_empty() {
        let queue = BoundedQueue::new(10);
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
        let queue = BoundedQueue::new(100);
        let caps = queue.capabilities();
        assert!(caps.is_bounded);
        assert_eq!(caps.capacity, Some(100));
        assert!(caps.is_lock_free);
        assert!(!caps.supports_priority);
        assert!(!caps.exact_size);
    }

    #[test]
    fn test_concurrent_bounded() {
        let queue = Arc::new(BoundedQueue::new(10));
        let num_jobs = 100;

        // Spawn sender thread
        let q_send = Arc::clone(&queue);
        let sender = thread::spawn(move || {
            for _ in 0..num_jobs {
                q_send.send(create_test_job()).unwrap();
            }
        });

        // Spawn receiver thread
        let q_recv = Arc::clone(&queue);
        let receiver = thread::spawn(move || {
            let mut received = 0;
            for _ in 0..num_jobs {
                q_recv.recv().unwrap();
                received += 1;
            }
            received
        });

        sender.join().unwrap();
        let received = receiver.join().unwrap();
        assert_eq!(received, num_jobs);
    }
}
