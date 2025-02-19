//! Priority-based thread pool implementation.
//!
//! This module provides a priority-based thread pool implementation that schedules
//! jobs based on their priority levels.

use std::fmt;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use std::cmp::Ordering as CmpOrdering;

use crossbeam_channel::{Sender, Receiver, unbounded};
use parking_lot::RwLock;

use crate::error::{Error, Result};
use crate::job::{Job, JobQueue};
use crate::thread_base::{ThreadWorker, BasicThreadWorker};

/// Job priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JobPriority {
    /// Lowest priority
    Lowest = 0,
    /// Low priority
    Low = 1,
    /// Normal priority
    Normal = 2,
    /// High priority
    High = 3,
    /// Highest priority
    Highest = 4,
}

impl fmt::Display for JobPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobPriority::Lowest => write!(f, "Lowest"),
            JobPriority::Low => write!(f, "Low"),
            JobPriority::Normal => write!(f, "Normal"),
            JobPriority::High => write!(f, "High"),
            JobPriority::Highest => write!(f, "Highest"),
        }
    }
}

/// A job with an associated priority level
pub struct PriorityJob {
    /// The job to execute
    job: Arc<dyn Job>,
    /// The priority of this job
    priority: JobPriority,
}

impl PriorityJob {
    /// Create a new priority job with the given job and priority
    pub fn new(job: impl Job + 'static, priority: JobPriority) -> Self {
        Self {
            job: Arc::new(job),
            priority,
        }
    }
    
    /// Get the priority of this job
    pub fn priority(&self) -> JobPriority {
        self.priority
    }
}

impl Job for PriorityJob {
    fn execute(&self) -> Result<()> {
        self.job.execute()
    }
    
    fn description(&self) -> String {
        format!("PriorityJob[{}]: {}", self.priority, self.job.description())
    }
    
    fn creation_time(&self) -> std::time::Instant {
        self.job.creation_time()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl fmt::Debug for PriorityJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriorityJob")
            .field("priority", &self.priority)
            .field("job", &self.job.description())
            .field("creation_time", &format!("{:?}", self.job.creation_time()))
            .finish()
    }
}

impl PartialEq for PriorityJob {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for PriorityJob {}

impl PartialOrd for PriorityJob {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriorityJob {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Higher priority jobs should come first
        other.priority.cmp(&self.priority)
    }
}

/// A queue for priority-based jobs
pub struct PriorityJobQueue {
    /// Channel sender for jobs
    sender: Sender<Arc<dyn Job>>,
    /// Channel receiver for jobs
    receiver: Receiver<Arc<dyn Job>>,
    /// Sorted buffer of jobs by priority
    buffer: RwLock<Vec<PriorityJob>>,
}

impl PriorityJobQueue {
    /// Create a new priority job queue
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self {
            sender,
            receiver,
            buffer: RwLock::new(Vec::new()),
        }
    }
    
    /// Enqueue a job with the specified priority
    pub fn enqueue_with_priority(&self, job: impl Job + 'static, priority: JobPriority) -> Result<()> {
        let priority_job = PriorityJob::new(job, priority);
        
        // Add to the sorted buffer
        let mut buffer = self.buffer.write();
        buffer.push(priority_job);
        buffer.sort();
        
        Ok(())
    }
}

impl Default for PriorityJobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue for PriorityJobQueue {
    fn enqueue(&self, job: Arc<dyn Job>) -> Result<()> {
        // Wrap in a PriorityJob with normal priority
        let priority_job = Arc::new(PriorityJob {
            job,
            priority: JobPriority::Normal,
        });
        
        self.sender.send(priority_job).map_err(|e| 
            Error::queue_error(format!("Failed to enqueue job: {}", e))
        )
    }
    
    fn dequeue(&self) -> Option<Arc<dyn Job>> {
        // First check our sorted buffer
        {
            let mut buffer = self.buffer.write();
            if !buffer.is_empty() {
                // Get highest priority job
                let job = buffer.remove(0);
                return Some(Arc::new(job));
            }
        }
        
        // If no jobs in buffer, check the channel
        self.receiver.try_recv().ok()
    }
    
    fn is_empty(&self) -> bool {
        let buffer_empty = self.buffer.read().is_empty();
        buffer_empty && self.receiver.is_empty()
    }
    
    fn len(&self) -> usize {
        let buffer_len = self.buffer.read().len();
        buffer_len + self.receiver.len()
    }
    
    fn clear(&self) {
        {
            let mut buffer = self.buffer.write();
            buffer.clear();
        }
        
        while self.receiver.try_recv().is_ok() {}
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl fmt::Debug for PriorityJobQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriorityJobQueue")
            .field("total_jobs", &self.len())
            .field("buffered_jobs", &self.buffer.read().len())
            .finish()
    }
}

/// A thread pool that processes jobs based on their priorities
pub struct PriorityThreadPool {
    /// Title for the thread pool
    title: String,
    
    /// Flag indicating if the pool is running
    running: Arc<AtomicBool>,
    
    /// Priority job queue used by all workers in the pool
    job_queue: Arc<PriorityJobQueue>,
    
    /// Worker threads
    workers: Mutex<Vec<Box<dyn ThreadWorker>>>,
    
    /// Default check interval for worker threads
    check_interval: Duration,
}

impl PriorityThreadPool {
    /// Create a new priority thread pool with the given title
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            running: Arc::new(AtomicBool::new(false)),
            job_queue: Arc::new(PriorityJobQueue::new()),
            workers: Mutex::new(Vec::new()),
            check_interval: Duration::from_millis(50),
        }
    }
    
    /// Create a new priority thread pool with the specified number of worker threads
    pub fn with_workers(num_workers: usize) -> Self {
        let pool = Self::new("priority_thread_pool");
        let _ = pool.add_workers(num_workers);
        pool
    }
    
    /// Add a specified number of worker threads to the pool
    pub fn add_workers(&self, count: usize) -> Result<()> {
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        for i in 0..count {
            let worker_title = format!("{}_worker_{}", self.title, workers.len() + i);
            let mut worker = Box::new(BasicThreadWorker::new(
                worker_title,
                self.job_queue.clone() as Arc<dyn JobQueue>,
                self.check_interval,
            ));
            
            if self.is_running() {
                worker.start()?;
            }
            
            workers.push(worker);
        }
        
        Ok(())
    }
    
    /// Add a custom worker to the pool
    pub fn add_worker(&self, mut worker: Box<dyn ThreadWorker>) -> Result<()> {
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        if self.is_running() {
            worker.start()?;
        }
        
        workers.push(worker);
        Ok(())
    }
    
    /// Submit a job with the specified priority to be executed by the thread pool
    pub fn submit_with_priority(
        &self, 
        job: impl Job + 'static, 
        priority: JobPriority
    ) -> Result<()> {
        if !self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Priority thread pool '{}' is not running", self.title)
            ));
        }
        
        self.job_queue.enqueue_with_priority(job, priority)
    }
    
    /// Submit a job with normal priority to be executed by the thread pool
    pub fn submit(&self, job: impl Job + 'static) -> Result<()> {
        self.submit_with_priority(job, JobPriority::Normal)
    }
    
    /// Start the thread pool and all worker threads
    pub fn start(&self) -> Result<()> {
        if self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Priority thread pool '{}' is already running", self.title)
            ));
        }
        
        self.running.store(true, Ordering::SeqCst);
        
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        for worker in workers.iter_mut() {
            worker.start()?;
        }
        
        Ok(())
    }
    
    /// Stop the thread pool and all worker threads
    pub fn stop(&self, wait_for_completion: bool) {
        self.running.store(false, Ordering::SeqCst);
        
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.iter_mut() {
                worker.stop(wait_for_completion);
            }
        }
    }
    
    /// Check if the thread pool is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    /// Get the number of workers in the pool
    pub fn worker_count(&self) -> usize {
        self.workers.lock().map(|w| w.len()).unwrap_or(0)
    }
    
    /// Get the job queue used by this thread pool
    pub fn job_queue(&self) -> Arc<PriorityJobQueue> {
        self.job_queue.clone()
    }
    
    /// Set the check interval for worker threads
    pub fn set_check_interval(&mut self, interval: Duration) {
        self.check_interval = interval;
    }
}

impl fmt::Debug for PriorityThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriorityThreadPool")
            .field("title", &self.title)
            .field("running", &self.is_running())
            .field("worker_count", &self.worker_count())
            .finish()
    }
}

impl fmt::Display for PriorityThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PriorityThreadPool '{}' (running: {}, workers: {})",
            self.title,
            if self.is_running() { "yes" } else { "no" },
            self.worker_count()
        )
    }
}

impl Drop for PriorityThreadPool {
    fn drop(&mut self) {
        self.stop(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::CallbackJob;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    
    #[test]
    fn test_job_priority() {
        assert!(JobPriority::Highest > JobPriority::High);
        assert!(JobPriority::High > JobPriority::Normal);
        assert!(JobPriority::Normal > JobPriority::Low);
        assert!(JobPriority::Low > JobPriority::Lowest);
    }
    
    #[test]
    fn test_priority_job() {
        let job = CallbackJob::new(|| Ok(()), "test job");
        let priority_job = PriorityJob::new(job, JobPriority::High);
        
        assert_eq!(priority_job.priority(), JobPriority::High);
        assert!(priority_job.description().contains("High"));
        assert!(priority_job.execute().is_ok());
    }
    
    #[test]
    fn test_priority_thread_pool() {
        let pool = PriorityThreadPool::with_workers(2);
        assert_eq!(pool.worker_count(), 2);
        
        assert!(!pool.is_running());
        assert!(pool.start().is_ok());
        assert!(pool.is_running());
        
        let counter = Arc::new(AtomicUsize::new(0));
        let high_counter = Arc::new(AtomicUsize::new(0));
        
        // Submit jobs with different priorities
        for i in 0..5 {
            let counter_clone = counter.clone();
            let job = CallbackJob::new(move || {
                std::thread::sleep(Duration::from_millis(10));
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }, format!("normal_job_{}", i));
            
            assert!(pool.submit(job).is_ok());
        }
        
        // Submit high priority job
        let high_counter_clone = high_counter.clone();
        let high_job = CallbackJob::new(move || {
            high_counter_clone.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }, "high_priority_job");
        
        assert!(pool.submit_with_priority(high_job, JobPriority::Highest).is_ok());
        
        // Wait for jobs to complete
        std::thread::sleep(Duration::from_millis(100));
        
        // Verify execution
        assert_eq!(counter.load(Ordering::SeqCst), 5);
        assert_eq!(high_counter.load(Ordering::SeqCst), 1);
        
        pool.stop(true);
        assert!(!pool.is_running());
    }
    
    #[test]
    fn test_priority_job_ordering() {
        let job1 = CallbackJob::new(|| Ok(()), "job1");
        let job2 = CallbackJob::new(|| Ok(()), "job2");
        
        let low_job = PriorityJob::new(job1, JobPriority::Low);
        let high_job = PriorityJob::new(job2, JobPriority::High);
        
        // Test Ord implementation - high priority comes before low
        assert!(high_job < low_job);
        
        // Create a vector and sort it
        let mut jobs = vec![low_job, high_job];
        jobs.sort();
        
        // After sorting, high priority should be first
        assert_eq!(jobs[0].priority(), JobPriority::High);
        assert_eq!(jobs[1].priority(), JobPriority::Low);
    }
}