//! Unbounded FIFO queue using crossbeam channels.

use super::{BoxedJobHolder, JobQueue, QueueCapabilities, QueueError, QueueResult};
use crate::core::BoxedJob;
use crossbeam::channel::{self, Receiver, Sender, TryRecvError, TrySendError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// An unbounded FIFO queue using crossbeam channels.
///
/// This is the default queue implementation, providing high-performance
/// lock-free operations for most use cases.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::queue::{ChannelQueue, JobQueue};
/// use rust_thread_system::core::ClosureJob;
///
/// let queue = ChannelQueue::unbounded();
/// let job = Box::new(ClosureJob::new(|| Ok(())));
/// queue.send(job).unwrap();
/// let received = queue.recv().unwrap();
/// ```
pub struct ChannelQueue {
    sender: Sender<BoxedJob>,
    receiver: Receiver<BoxedJob>,
    closed: AtomicBool,
}

impl ChannelQueue {
    /// Creates a new unbounded channel queue.
    pub fn unbounded() -> Self {
        let (sender, receiver) = channel::unbounded();
        Self {
            sender,
            receiver,
            closed: AtomicBool::new(false),
        }
    }
}

impl JobQueue for ChannelQueue {
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
        QueueCapabilities::unbounded_channel()
    }
}

// Safety: ChannelQueue is Send + Sync because:
// - Sender and Receiver from crossbeam are Send + Sync
// - AtomicBool is Send + Sync
unsafe impl Send for ChannelQueue {}
unsafe impl Sync for ChannelQueue {}

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
    fn test_unbounded_send_recv() {
        let queue = ChannelQueue::unbounded();
        queue.send(create_test_job()).unwrap();
        let job = queue.recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
    }

    #[test]
    fn test_try_send_recv() {
        let queue = ChannelQueue::unbounded();
        queue.try_send(create_test_job()).unwrap();
        let job = queue.try_recv().unwrap();
        assert_eq!(job.job_type(), "ClosureJob");
    }

    #[test]
    fn test_try_recv_empty() {
        let queue = ChannelQueue::unbounded();
        match queue.try_recv() {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error"),
        }
    }

    #[test]
    fn test_recv_timeout() {
        let queue = ChannelQueue::unbounded();
        let result = queue.recv_timeout(Duration::from_millis(10));
        match result {
            Err(QueueError::Empty) => {}
            _ => panic!("expected Empty error on timeout"),
        }
    }

    #[test]
    fn test_send_timeout() {
        let queue = ChannelQueue::unbounded();
        // Unbounded queue should never timeout on send
        queue
            .send_timeout(create_test_job(), Duration::from_millis(10))
            .unwrap();
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_close() {
        let queue = ChannelQueue::unbounded();
        assert!(!queue.is_closed());
        queue.close();
        assert!(queue.is_closed());

        // Send should fail after close
        match queue.send(create_test_job()) {
            Err(QueueError::Closed(_)) => {}
            _ => panic!("expected Closed error"),
        }
    }

    #[test]
    fn test_len_and_is_empty() {
        let queue = ChannelQueue::unbounded();
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
        let queue = ChannelQueue::unbounded();
        let caps = queue.capabilities();
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
    fn test_concurrent_send_recv() {
        let queue = Arc::new(ChannelQueue::unbounded());
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
}
