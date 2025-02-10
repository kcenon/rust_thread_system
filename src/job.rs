//! Job abstractions and implementations.
//!
//! This module defines the job traits and implementations that represent units of work
//! to be executed by worker threads in the thread system.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::backoff::RetryPolicy;
use crate::error::{Error, Result};

/// Job trait representing a unit of work that can be executed by a worker thread.
pub trait Job: Send + Sync + fmt::Debug {
    /// Execute the job and return a result.
    fn execute(&self) -> Result<()>;
    
    /// Returns a string description of this job.
    fn description(&self) -> String;
    
    /// Returns when the job was created.
    fn creation_time(&self) -> Instant;
    
    /// Return self as a trait object for downcasting.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A basic job implementation that wraps a closure.
pub struct CallbackJob<F>
where
    F: Fn() -> Result<()> + Send + Sync + 'static,
{
    /// The callback function to execute.
    callback: F,
    
    /// Description of the job.
    description: String,
    
    /// When the job was created.
    creation_time: Instant,
}

impl<F> CallbackJob<F>
where
    F: Fn() -> Result<()> + Send + Sync + 'static,
{
    /// Create a new callback job with the given function and description.
    pub fn new(callback: F, description: impl Into<String>) -> Self {
        Self {
            callback,
            description: description.into(),
            creation_time: Instant::now(),
        }
    }
}

impl<F> Job for CallbackJob<F>
where
    F: Fn() -> Result<()> + Send + Sync + 'static,
{
    fn execute(&self) -> Result<()> {
        (self.callback)()
    }
    
    fn description(&self) -> String {
        self.description.clone()
    }
    
    fn creation_time(&self) -> Instant {
        self.creation_time
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<F> fmt::Debug for CallbackJob<F>
where
    F: Fn() -> Result<()> + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackJob")
            .field("description", &self.description)
            .field("creation_time", &format!("{:?}", self.creation_time))
            .finish()
    }
}

/// A job queue interface for storing and retrieving jobs.
pub trait JobQueue: Send + Sync + fmt::Debug {
    /// Add a job to the queue.
    fn enqueue(&self, job: Arc<dyn Job>) -> Result<()>;
    
    /// Get the next job from the queue, if available.
    fn dequeue(&self) -> Option<Arc<dyn Job>>;
    
    /// Check if the queue is empty.
    fn is_empty(&self) -> bool;
    
    /// Get the number of jobs in the queue.
    fn len(&self) -> usize;
    
    /// Clear all jobs from the queue.
    fn clear(&self);
    
    /// Return self as a trait object for downcasting.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A basic job queue implementation using crossbeam channels.
pub struct BasicJobQueue {
    /// The channel for sending jobs.
    sender: crossbeam_channel::Sender<Arc<dyn Job>>,
    
    /// The channel for receiving jobs.
    receiver: crossbeam_channel::Receiver<Arc<dyn Job>>,
    
    /// Queue for jobs that need to be retried later.
    retry_queue: Mutex<Vec<(Arc<dyn Job>, Instant)>>,
}

impl BasicJobQueue {
    /// Create a new basic job queue.
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self { 
            sender, 
            receiver, 
            retry_queue: Mutex::new(Vec::new()),
        }
    }
    
    /// Process the retry queue and requeue jobs that are ready to be retried.
    pub fn process_retry_queue(&self) -> Result<()> {
        let mut retry_queue = self.retry_queue.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock retry queue: {}", e))
        )?;
        
        let now = Instant::now();
        let mut i = 0;
        
        while i < retry_queue.len() {
            let (job, retry_time) = &retry_queue[i];
            
            if now >= *retry_time {
                // This job is ready to be retried
                let job = job.clone();
                self.sender.send(job).map_err(|e| 
                    Error::queue_error(format!("Failed to requeue job: {}", e))
                )?;
                
                // Remove from retry queue using swap_remove for efficiency
                retry_queue.swap_remove(i);
            } else {
                // Not ready yet, move to next job
                i += 1;
            }
        }
        
        Ok(())
    }
    
    /// Add a job to the retry queue.
    pub fn enqueue_retry(&self, job: Arc<dyn Job>, retry_after: Instant) -> Result<()> {
        let mut retry_queue = self.retry_queue.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock retry queue: {}", e))
        )?;
        
        retry_queue.push((job, retry_after));
        Ok(())
    }
    
    /// Get the number of jobs in the retry queue.
    pub fn retry_queue_len(&self) -> usize {
        self.retry_queue.lock().map(|q| q.len()).unwrap_or(0)
    }
}

impl Default for BasicJobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue for BasicJobQueue {
    fn enqueue(&self, job: Arc<dyn Job>) -> Result<()> {
        self.sender.send(job).map_err(|e| 
            Error::queue_error(format!("Failed to enqueue job: {}", e))
        )
    }
    
    fn dequeue(&self) -> Option<Arc<dyn Job>> {
        // First process any jobs that are ready to be retried
        let _ = self.process_retry_queue();
        
        // Then try to get a job from the main queue
        self.receiver.try_recv().ok()
    }
    
    fn is_empty(&self) -> bool {
        self.receiver.is_empty() && self.retry_queue_len() == 0
    }
    
    fn len(&self) -> usize {
        self.receiver.len() + self.retry_queue_len()
    }
    
    fn clear(&self) {
        while self.receiver.try_recv().is_ok() {}
        
        if let Ok(mut retry_queue) = self.retry_queue.lock() {
            retry_queue.clear();
        }
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl fmt::Debug for BasicJobQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasicJobQueue")
            .field("queue_length", &self.receiver.len())
            .field("retry_queue_length", &self.retry_queue_len())
            .field("total_length", &self.len())
            .finish()
    }
}

/// A job that can be retried with a backoff strategy if it fails.
pub struct RetryableJob<J: Job> {
    /// The inner job to execute.
    inner: J,
    
    /// The retry policy to use if the job fails.
    retry_policy: Mutex<RetryPolicy>,
    
    /// The maximum number of attempts before failing.
    max_attempts: u32,
    
    /// The current attempt number.
    current_attempt: Mutex<u32>,
}

impl<J: Job> RetryableJob<J> {
    /// Create a new retryable job with the given inner job and retry policy.
    pub fn new(inner: J, retry_policy: RetryPolicy) -> Self {
        let max_attempts = retry_policy.max_attempts();
        Self {
            inner,
            retry_policy: Mutex::new(retry_policy),
            max_attempts,
            current_attempt: Mutex::new(0),
        }
    }
    
    /// Get the inner job.
    pub fn inner(&self) -> &J {
        &self.inner
    }
    
    /// Get the current attempt number.
    pub fn current_attempt(&self) -> u32 {
        *self.current_attempt.lock().unwrap()
    }
    
    /// Reset the retry state.
    pub fn reset(&self) {
        let mut attempt = self.current_attempt.lock().unwrap();
        *attempt = 0;
        
        let mut policy = self.retry_policy.lock().unwrap();
        policy.reset();
    }
    
    /// Check if the job has reached its maximum number of attempts.
    pub fn is_exhausted(&self) -> bool {
        self.current_attempt() >= self.max_attempts
    }
    
    /// Check if the job is ready to be retried.
    pub fn is_ready_for_retry(&self) -> bool {
        let policy = self.retry_policy.lock().unwrap();
        policy.is_ready()
    }
}

impl<J: Job + 'static> Job for RetryableJob<J> {
    fn execute(&self) -> Result<()> {
        // Increment the attempt counter
        let mut attempt = self.current_attempt.lock().unwrap();
        *attempt += 1;
        let current = *attempt;
        drop(attempt); // Release the lock
        
        // Execute the inner job
        match self.inner.execute() {
            Ok(()) => Ok(()),
            Err(e) => {
                // Job failed, handle retry logic
                let mut policy = self.retry_policy.lock().unwrap();
                
                if current >= self.max_attempts {
                    // No more retries
                    return Err(Error::job_error(format!(
                        "Job '{}' failed after {} attempts: {}",
                        self.description(),
                        current,
                        e
                    )));
                }
                
                // Record the failure and schedule retry
                if policy.record_failure() {
                    // This job should be retried later
                    Err(Error::job_error(format!(
                        "Job '{}' failed on attempt {}/{}, will retry after {:?}: {}",
                        self.description(),
                        current,
                        self.max_attempts,
                        policy.time_remaining().unwrap_or_default(),
                        e
                    )))
                } else {
                    // No more retries according to policy
                    Err(Error::job_error(format!(
                        "Job '{}' failed and retry policy exhausted after {} attempts: {}",
                        self.description(),
                        current,
                        e
                    )))
                }
            }
        }
    }
    
    fn description(&self) -> String {
        format!("Retryable({})", self.inner.description())
    }
    
    fn creation_time(&self) -> Instant {
        self.inner.creation_time()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<J: Job> fmt::Debug for RetryableJob<J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetryableJob")
            .field("inner", &self.inner)
            .field("max_attempts", &self.max_attempts)
            .field("current_attempt", &self.current_attempt())
            .finish()
    }
}

/// A batch of jobs for efficient enqueueing.
#[derive(Debug)]
pub struct JobBatch {
    /// The jobs to be executed.
    jobs: Vec<Arc<dyn Job>>,
    
    /// A description of the batch.
    description: String,
}

impl JobBatch {
    /// Create a new job batch.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            jobs: Vec::new(),
            description: description.into(),
        }
    }
    
    /// Add a job to the batch.
    pub fn add(&mut self, job: impl Job + 'static) -> &mut Self {
        self.jobs.push(Arc::new(job));
        self
    }
    
    /// Get the number of jobs in the batch.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }
    
    /// Check if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
    
    /// Get a description of the batch.
    pub fn description(&self) -> &str {
        &self.description
    }
    
    /// Get the jobs in the batch.
    pub fn jobs(&self) -> &[Arc<dyn Job>] {
        &self.jobs
    }
    
    /// Consume the batch and return the jobs.
    pub fn into_jobs(self) -> Vec<Arc<dyn Job>> {
        self.jobs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_callback_job() {
        let job = CallbackJob::new(|| Ok(()), "test job");
        assert_eq!(job.description(), "test job");
        assert!(job.execute().is_ok());
        
        let job = CallbackJob::new(|| Err(Error::job_error("error")), "failing job");
        assert!(job.execute().is_err());
    }
    
    #[test]
    fn test_basic_job_queue() {
        let queue = BasicJobQueue::new();
        assert!(queue.is_empty());
        
        let job = Arc::new(CallbackJob::new(|| Ok(()), "test job"));
        assert!(queue.enqueue(job.clone()).is_ok());
        
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
        
        let dequeued = queue.dequeue();
        assert!(dequeued.is_some());
        assert!(queue.is_empty());
    }
    
    #[test]
    fn test_job_batch() {
        let mut batch = JobBatch::new("test batch");
        assert!(batch.is_empty());
        
        batch.add(CallbackJob::new(|| Ok(()), "job 1"))
             .add(CallbackJob::new(|| Ok(()), "job 2"));
        
        assert_eq!(batch.len(), 2);
        assert!(!batch.is_empty());
        assert_eq!(batch.description(), "test batch");
    }
}