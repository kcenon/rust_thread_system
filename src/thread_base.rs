//! Thread base traits and implementations.
//!
//! This module defines the core abstractions for thread management in the system,
//! providing traits and implementations for thread workers and thread conditions.

use std::fmt;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::job::{JobQueue, BasicJobQueue};

/// Enum representing the possible thread conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadCondition {
    /// Thread is not started
    NotStarted,
    /// Thread is running
    Running,
    /// Thread is stopping
    Stopping,
    /// Thread is stopped
    Stopped,
}

impl fmt::Display for ThreadCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThreadCondition::NotStarted => write!(f, "Not Started"),
            ThreadCondition::Running => write!(f, "Running"),
            ThreadCondition::Stopping => write!(f, "Stopping"),
            ThreadCondition::Stopped => write!(f, "Stopped"),
        }
    }
}

/// Trait defining the interface for a thread worker.
pub trait ThreadWorker: Send + Sync + fmt::Debug {
    /// Start the worker thread.
    fn start(&mut self) -> Result<()>;
    
    /// Stop the worker thread.
    ///
    /// If `wait_for_completion` is true, this method will block until the
    /// worker thread has fully stopped. Otherwise, it will request the thread
    /// to stop and return immediately.
    fn stop(&mut self, wait_for_completion: bool);
    
    /// Check if the worker thread is running.
    fn is_running(&self) -> bool;
    
    /// Get a string representation of this worker.
    fn to_string(&self) -> String;
}

/// A basic implementation of a thread worker.
pub struct BasicThreadWorker {
    /// Title for the thread
    title: String,
    
    /// Flag indicating if the thread is running
    running: Arc<AtomicBool>,
    
    /// Handle to the thread, if started
    handle: Option<JoinHandle<()>>,
    
    /// Job queue to pull work from
    job_queue: Arc<dyn JobQueue>,
    
    /// Interval to check for new jobs when idle
    check_interval: Duration,
    
    /// Current condition of the thread
    condition: ThreadCondition,
}

impl BasicThreadWorker {
    /// Create a new basic thread worker with the given job queue.
    pub fn new(
        title: impl Into<String>,
        job_queue: Arc<dyn JobQueue>,
        check_interval: Duration,
    ) -> Self {
        Self {
            title: title.into(),
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
            job_queue,
            check_interval,
            condition: ThreadCondition::NotStarted,
        }
    }
    
    /// Set the job queue for this worker.
    pub fn set_job_queue(&mut self, job_queue: Arc<dyn JobQueue>) {
        self.job_queue = job_queue;
    }
    
    /// Set the check interval for this worker.
    pub fn set_check_interval(&mut self, interval: Duration) {
        self.check_interval = interval;
    }
    
    /// Get the current thread condition
    pub fn condition(&self) -> ThreadCondition {
        self.condition
    }
}

impl ThreadWorker for BasicThreadWorker {
    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Err(Error::thread_error(
                format!("Worker '{}' is already running", self.title)
            ));
        }
        
        self.running.store(true, Ordering::SeqCst);
        self.condition = ThreadCondition::Running;
        
        let running = self.running.clone();
        let job_queue = self.job_queue.clone();
        let thread_title_for_thread = self.title.clone();
        let thread_title_for_error = self.title.clone();
        let check_interval = self.check_interval;
        
        let handle = thread::Builder::new()
            .name(thread_title_for_thread.clone())
            .spawn(move || {
                while running.load(Ordering::SeqCst) {
                    // Try to get a job from the queue
                    if let Some(job) = job_queue.dequeue() {
                        match job.execute() {
                            Ok(()) => {
                                // Job completed successfully
                            },
                            Err(e) => {
                                let _ = crate::log_error!("Job execution error in worker '{}': {}", thread_title_for_thread, e);
                                
                                // Check if this job needs to be retried later
                                if let Some(basic_queue) = job_queue.as_any().downcast_ref::<BasicJobQueue>() {
                                    // Try to extract retry information from the error message to determine next retry time
                                    // This is a basic approach; in a more sophisticated system we might want to use special
                                    // error types or job traits to handle this more elegantly
                                    if e.to_string().contains("will retry after") {
                                        // Get the current time plus a reasonable default backoff
                                        let retry_time = Instant::now() + Duration::from_secs(5);
                                        
                                        if let Err(retry_err) = basic_queue.enqueue_retry(job.clone(), retry_time) {
                                            let _ = crate::log_error!("Failed to schedule job for retry: {}", retry_err);
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        // No job available, sleep for a bit
                        thread::sleep(check_interval);
                    }
                }
            })
            .map_err(|e| Error::thread_error(
                format!("Failed to spawn worker thread '{}': {}", thread_title_for_error, e)
            ))?;
        
        self.handle = Some(handle);
        Ok(())
    }
    
    fn stop(&mut self, wait_for_completion: bool) {
        if !self.is_running() {
            return;
        }
        
        self.condition = ThreadCondition::Stopping;
        self.running.store(false, Ordering::SeqCst);
        
        if wait_for_completion {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
        
        self.condition = ThreadCondition::Stopped;
    }
    
    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    fn to_string(&self) -> String {
        format!("BasicThreadWorker '{}' ({})",
            self.title,
            if self.is_running() { "running" } else { "stopped" }
        )
    }
}

impl fmt::Debug for BasicThreadWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasicThreadWorker")
            .field("title", &self.title)
            .field("running", &self.is_running())
            .field("condition", &self.condition)
            .finish()
    }
}

impl Drop for BasicThreadWorker {
    fn drop(&mut self) {
        self.stop(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{BasicJobQueue, CallbackJob};
    use std::sync::Arc;
    use std::time::Duration;
    
    #[test]
    fn test_thread_condition_display() {
        assert_eq!(format!("{}", ThreadCondition::NotStarted), "Not Started");
        assert_eq!(format!("{}", ThreadCondition::Running), "Running");
        assert_eq!(format!("{}", ThreadCondition::Stopping), "Stopping");
        assert_eq!(format!("{}", ThreadCondition::Stopped), "Stopped");
    }
    
    #[test]
    fn test_basic_thread_worker() {
        let job_queue = Arc::new(BasicJobQueue::new());
        let mut worker = BasicThreadWorker::new(
            "test_worker", 
            job_queue.clone(), 
            Duration::from_millis(10)
        );
        
        // Worker should start not running
        assert!(!worker.is_running());
        assert_eq!(worker.condition(), ThreadCondition::NotStarted);
        
        // Start the worker
        assert!(worker.start().is_ok());
        assert!(worker.is_running());
        assert_eq!(worker.condition(), ThreadCondition::Running);
        
        // Queue a job
        let job = Arc::new(CallbackJob::new(|| {
            // Simple test job
            Ok(())
        }, "test_job"));
        
        assert!(job_queue.enqueue(job).is_ok());
        
        // Let the worker process the job
        thread::sleep(Duration::from_millis(20));
        
        // Stop the worker
        worker.stop(true);
        assert!(!worker.is_running());
        assert_eq!(worker.condition(), ThreadCondition::Stopped);
    }
    
    #[test]
    fn test_thread_worker_to_string() {
        let job_queue = Arc::new(BasicJobQueue::new());
        let worker = BasicThreadWorker::new(
            "test_worker", 
            job_queue, 
            Duration::from_millis(10)
        );
        
        assert!(worker.to_string().contains("test_worker"));
        assert!(worker.to_string().contains("stopped"));
    }
}