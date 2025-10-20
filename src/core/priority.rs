//! Priority-based job scheduling
//!
//! This module provides priority levels for jobs and a priority queue implementation.
//!
//! **Status**: Experimental - Not yet integrated with ThreadPool
//!
//! Enable with the `priority-scheduling` feature:
//! ```toml
//! rust_thread_system = { version = "0.1", features = ["priority-scheduling"] }
//! ```

#[cfg(feature = "priority-scheduling")]
use super::job::BoxedJob;
#[cfg(feature = "priority-scheduling")]
use std::cmp::Ordering;
#[cfg(feature = "priority-scheduling")]
use std::collections::BinaryHeap;

/// Job priority levels (higher number = higher priority)
#[cfg(feature = "priority-scheduling")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Priority {
    /// Lowest priority - background tasks
    Low = 1,
    /// Normal priority - default for most tasks
    #[default]
    Normal = 5,
    /// High priority - important tasks
    High = 8,
    /// Critical priority - must be executed ASAP
    Critical = 10,
}

#[cfg(feature = "priority-scheduling")]
impl Priority {
    /// Get the numeric value of the priority
    pub fn value(&self) -> u8 {
        *self as u8
    }
}

/// A job with an associated priority
#[cfg(feature = "priority-scheduling")]
pub struct PriorityJob {
    pub(crate) priority: Priority,
    #[allow(dead_code)] // Will be used when integrated with ThreadPool
    pub(crate) job: BoxedJob,
    /// Sequence number for FIFO ordering within same priority
    pub(crate) sequence: u64,
}

#[cfg(feature = "priority-scheduling")]
impl PriorityJob {
    /// Create a new priority job
    pub fn new(priority: Priority, job: BoxedJob, sequence: u64) -> Self {
        Self {
            priority,
            job,
            sequence,
        }
    }

    /// Get the priority of this job
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Get the sequence number
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Ordering for priority jobs in the heap
/// Higher priority comes first; if equal, earlier sequence (FIFO)
#[cfg(feature = "priority-scheduling")]
impl PartialEq for PriorityJob {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}

#[cfg(feature = "priority-scheduling")]
impl Eq for PriorityJob {}

#[cfg(feature = "priority-scheduling")]
impl PartialOrd for PriorityJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "priority-scheduling")]
impl Ord for PriorityJob {
    fn cmp(&self, other: &Self) -> Ordering {
        // First compare by priority (higher is better)
        match self.priority.cmp(&other.priority) {
            Ordering::Equal => {
                // If priorities are equal, earlier sequence comes first (FIFO)
                // Reverse the comparison because BinaryHeap is a max-heap
                other.sequence.cmp(&self.sequence)
            }
            other => other,
        }
    }
}

/// Thread-safe priority queue for jobs
#[cfg(feature = "priority-scheduling")]
pub struct PriorityQueue {
    heap: BinaryHeap<PriorityJob>,
}

#[cfg(feature = "priority-scheduling")]
impl PriorityQueue {
    /// Create a new empty priority queue
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    /// Create a new priority queue with the specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(capacity),
        }
    }

    /// Push a job onto the queue
    pub fn push(&mut self, job: PriorityJob) {
        self.heap.push(job);
    }

    /// Pop the highest priority job from the queue
    pub fn pop(&mut self) -> Option<PriorityJob> {
        self.heap.pop()
    }

    /// Get the number of jobs in the queue
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Clear all jobs from the queue
    pub fn clear(&mut self) {
        self.heap.clear();
    }
}

#[cfg(feature = "priority-scheduling")]
impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, feature = "priority-scheduling"))]
mod tests {
    use super::*;
    use crate::core::ClosureJob;

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
    }

    #[test]
    fn test_priority_value() {
        assert_eq!(Priority::Low.value(), 1);
        assert_eq!(Priority::Normal.value(), 5);
        assert_eq!(Priority::High.value(), 8);
        assert_eq!(Priority::Critical.value(), 10);
    }

    #[test]
    fn test_priority_default() {
        assert_eq!(Priority::default(), Priority::Normal);
    }

    #[test]
    fn test_priority_queue_ordering() {
        let mut queue = PriorityQueue::new();

        // Add jobs with different priorities
        queue.push(PriorityJob::new(
            Priority::Low,
            Box::new(ClosureJob::new(|| Ok(()))),
            0,
        ));
        queue.push(PriorityJob::new(
            Priority::High,
            Box::new(ClosureJob::new(|| Ok(()))),
            1,
        ));
        queue.push(PriorityJob::new(
            Priority::Normal,
            Box::new(ClosureJob::new(|| Ok(()))),
            2,
        ));
        queue.push(PriorityJob::new(
            Priority::Critical,
            Box::new(ClosureJob::new(|| Ok(()))),
            3,
        ));

        // Pop should return in priority order
        assert_eq!(queue.pop().unwrap().priority(), Priority::Critical);
        assert_eq!(queue.pop().unwrap().priority(), Priority::High);
        assert_eq!(queue.pop().unwrap().priority(), Priority::Normal);
        assert_eq!(queue.pop().unwrap().priority(), Priority::Low);
        assert!(queue.pop().is_none());
    }

    #[test]
    fn test_priority_queue_fifo_within_priority() {
        let mut queue = PriorityQueue::new();

        // Add multiple jobs with same priority
        queue.push(PriorityJob::new(
            Priority::Normal,
            Box::new(ClosureJob::new(|| Ok(()))),
            1,
        ));
        queue.push(PriorityJob::new(
            Priority::Normal,
            Box::new(ClosureJob::new(|| Ok(()))),
            2,
        ));
        queue.push(PriorityJob::new(
            Priority::Normal,
            Box::new(ClosureJob::new(|| Ok(()))),
            3,
        ));

        // Should come out in FIFO order (sequence 1, 2, 3)
        assert_eq!(queue.pop().unwrap().sequence(), 1);
        assert_eq!(queue.pop().unwrap().sequence(), 2);
        assert_eq!(queue.pop().unwrap().sequence(), 3);
    }

    #[test]
    fn test_priority_queue_operations() {
        let mut queue = PriorityQueue::new();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        queue.push(PriorityJob::new(
            Priority::Normal,
            Box::new(ClosureJob::new(|| Ok(()))),
            1,
        ));

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);

        queue.clear();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }
}
