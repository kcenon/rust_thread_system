//! Thread pool implementation for managing worker threads.
//!
//! This module provides a thread pool implementation that manages worker threads
//! and distributes jobs among them efficiently.

use std::fmt;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::job::{Job, JobQueue, BasicJobQueue, JobBatch};
use crate::thread_base::{ThreadWorker, BasicThreadWorker};
use crate::log_error;

/// Thread pool that manages worker threads and distributes jobs.
pub struct ThreadPool {
    /// Title for the thread pool
    title: String,
    
    /// Flag indicating if the pool is running
    running: Arc<AtomicBool>,
    
    /// Job queue used by all workers in the pool
    job_queue: Arc<dyn JobQueue>,
    
    /// Worker threads
    workers: Mutex<Vec<Box<dyn ThreadWorker>>>,
    
    /// Default check interval for worker threads
    check_interval: Duration,
    
    /// Flag to indicate whether the pool should automatically shut down when all jobs are completed
    auto_shutdown: Arc<AtomicBool>,
    
    /// Flag to track if all workers are idle
    all_workers_idle: Arc<AtomicBool>,
}

// We need to implement Clone for thread pools
impl Clone for ThreadPool {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            running: self.running.clone(),
            job_queue: self.job_queue.clone(),
            workers: Mutex::new(Vec::new()), // Don't clone workers
            check_interval: self.check_interval,
            auto_shutdown: self.auto_shutdown.clone(),
            all_workers_idle: self.all_workers_idle.clone(),
        }
    }
}

impl ThreadPool {
    /// Create a new thread pool with the given title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            running: Arc::new(AtomicBool::new(false)),
            job_queue: Arc::new(BasicJobQueue::new()),
            workers: Mutex::new(Vec::new()),
            check_interval: Duration::from_millis(50),
            auto_shutdown: Arc::new(AtomicBool::new(false)),
            all_workers_idle: Arc::new(AtomicBool::new(false)),
        }
    }
    
    /// Create a new thread pool with the specified number of worker threads.
    pub fn with_workers(num_workers: usize) -> Self {
        let pool = Self::new("thread_pool");
        let _ = pool.add_workers(num_workers);
        pool
    }
    
    /// Create a new thread pool with the specified number of worker threads 
    /// that automatically shuts down after all jobs are completed.
    pub fn with_auto_shutdown(num_workers: usize) -> Self {
        let mut pool = Self::with_workers(num_workers);
        pool.enable_auto_shutdown();
        pool
    }
    
    /// Add a specified number of worker threads to the pool.
    pub fn add_workers(&self, count: usize) -> Result<()> {
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        for i in 0..count {
            let worker_title = format!("{}_worker_{}", self.title, workers.len() + i);
            let mut worker = Box::new(BasicThreadWorker::new(
                worker_title,
                self.job_queue.clone(),
                self.check_interval,
            ));
            
            if self.is_running() {
                if let Err(e) = worker.start() {
                    let _ = log_error!("Failed to start worker {}: {}", i, e);
                    // Continue with other workers
                }
            }
            
            workers.push(worker);
        }
        
        Ok(())
    }
    
    /// Add a custom worker to the pool.
    pub fn add_worker(&self, mut worker: Box<dyn ThreadWorker>) -> Result<()> {
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        if self.is_running() {
            if let Err(e) = worker.start() {
                let _ = log_error!("Failed to start custom worker: {}", e);
                return Err(e);
            }
        }
        
        workers.push(worker);
        Ok(())
    }
    
    /// Start the thread pool and all worker threads.
    pub fn start(&self) -> Result<()> {
        if self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Thread pool '{}' is already running", self.title)
            ));
        }
        
        self.running.store(true, Ordering::SeqCst);
        self.all_workers_idle.store(false, Ordering::SeqCst);
        
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        for (i, worker) in workers.iter_mut().enumerate() {
            if let Err(e) = worker.start() {
                let _ = log_error!("Failed to start worker {} during pool start: {}", i, e);
                // Continue with other workers
            }
        }
        
        // If auto-shutdown is enabled, start a monitoring thread to check for completion
        if self.is_auto_shutdown_enabled() {
            self.start_auto_shutdown_monitor();
        }
        
        Ok(())
    }
    
    /// Start a thread that monitors for job completion and triggers auto-shutdown
    fn start_auto_shutdown_monitor(&self) {
        let running = self.running.clone();
        let job_queue = self.job_queue.clone();
        let auto_shutdown = self.auto_shutdown.clone();
        let all_workers_idle = self.all_workers_idle.clone();
        let thread_pool_title = self.title.clone();
        let check_interval = self.check_interval;
        
        // Spawn a monitoring thread that periodically checks if all jobs are completed
        let _ = std::thread::Builder::new()
            .name(format!("{}_auto_shutdown_monitor", self.title))
            .spawn(move || {
                let mut idle_count = 0;
                
                while running.load(Ordering::SeqCst) {
                    // Check if auto-shutdown has been disabled
                    if !auto_shutdown.load(Ordering::SeqCst) {
                        std::thread::sleep(check_interval);
                        continue;
                    }
                    
                    // Check if job queue is empty
                    if job_queue.is_empty() {
                        idle_count += 1;
                        
                        // After seeing the queue empty for several consecutive checks,
                        // consider the pool idle and ready to shut down
                        if idle_count >= 3 {
                            let _ = crate::log_info!(
                                "Auto-shutdown triggered for thread pool '{}' - all jobs completed",
                                thread_pool_title
                            );
                            
                            // Mark all workers as idle
                            all_workers_idle.store(true, Ordering::SeqCst);
                            
                            // Trigger shutdown
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    } else {
                        // Reset idle count if queue isn't empty
                        idle_count = 0;
                    }
                    
                    // Wait before checking again
                    std::thread::sleep(check_interval);
                }
            });
    }
    
    /// Submit a job to be executed by the thread pool.
    pub fn submit(&self, job: impl Job + 'static) -> Result<()> {
        if !self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Thread pool '{}' is not running", self.title)
            ));
        }
        
        self.job_queue.enqueue(Arc::new(job))
    }
    
    /// Submit a batch of jobs to be executed by the thread pool.
    pub fn submit_batch(&self, batch: JobBatch) -> Result<()> {
        if !self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Thread pool '{}' is not running", self.title)
            ));
        }
        
        for job in batch.into_jobs() {
            if let Err(e) = self.job_queue.enqueue(job) {
                let _ = log_error!("Failed to enqueue job from batch: {}", e);
                return Err(e);
            }
        }
        
        Ok(())
    }
    
    /// Stop the thread pool and all worker threads.
    pub fn stop(&self, wait_for_completion: bool) {
        // Log stopping message
        let _ = crate::log_info!("Stopping thread pool '{}'...", self.title);
        
        // First set the running flag to false
        self.running.store(false, Ordering::SeqCst);
        
        // Signal threads to stop
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.iter_mut() {
                worker.stop(wait_for_completion);
            }
        } else {
            let _ = crate::log_error!("Failed to lock workers mutex when stopping thread pool");
        }
        
        let _ = crate::log_info!("Thread pool '{}' stopped", self.title);
    }
    
    /// Check if the thread pool is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    /// Get the number of workers in the pool.
    pub fn worker_count(&self) -> usize {
        self.workers.lock().map(|w| w.len()).unwrap_or(0)
    }
    
    /// Get the job queue used by this thread pool.
    pub fn job_queue(&self) -> Arc<dyn JobQueue> {
        self.job_queue.clone()
    }
    
    /// Set the check interval for worker threads.
    pub fn set_check_interval(&mut self, interval: Duration) {
        self.check_interval = interval;
    }
    
    /// Enable automatic shutdown when all jobs are completed.
    pub fn enable_auto_shutdown(&mut self) {
        self.auto_shutdown.store(true, Ordering::SeqCst);
    }
    
    /// Disable automatic shutdown.
    pub fn disable_auto_shutdown(&mut self) {
        self.auto_shutdown.store(false, Ordering::SeqCst);
    }
    
    /// Check if automatic shutdown is enabled.
    pub fn is_auto_shutdown_enabled(&self) -> bool {
        self.auto_shutdown.load(Ordering::SeqCst)
    }
    
}

impl fmt::Debug for ThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThreadPool")
            .field("title", &self.title)
            .field("running", &self.is_running())
            .field("worker_count", &self.worker_count())
            .field("auto_shutdown", &self.is_auto_shutdown_enabled())
            .field("all_workers_idle", &self.all_workers_idle.load(Ordering::SeqCst))
            .finish()
    }
}

impl fmt::Display for ThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ThreadPool '{}' (running: {}, workers: {}, auto-shutdown: {})",
            self.title,
            if self.is_running() { "yes" } else { "no" },
            self.worker_count(),
            if self.is_auto_shutdown_enabled() { "enabled" } else { "disabled" }
        )
    }
}

impl Drop for ThreadPool {
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
    fn test_thread_pool_creation() {
        let pool = ThreadPool::new("test_pool");
        assert_eq!(pool.worker_count(), 0);
        assert!(!pool.is_running());
        
        let pool = ThreadPool::with_workers(4);
        assert_eq!(pool.worker_count(), 4);
    }
    
    #[test]
    fn test_thread_pool_start_stop() {
        let pool = ThreadPool::with_workers(2);
        
        assert!(!pool.is_running());
        assert!(pool.start().is_ok());
        assert!(pool.is_running());
        
        // Starting again should fail
        assert!(pool.start().is_err());
        
        pool.stop(true);
        assert!(!pool.is_running());
    }
    
    #[test]
    fn test_thread_pool_job_execution() {
        let pool = ThreadPool::with_workers(2);
        assert!(pool.start().is_ok());
        
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        
        // Submit a job
        let job = CallbackJob::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }, "test_job");
        
        assert!(pool.submit(job).is_ok());
        
        // Give time for the job to complete
        std::thread::sleep(Duration::from_millis(50));
        
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        
        // Clean up
        pool.stop(true);
    }
    
    #[test]
    fn test_thread_pool_batch_submission() {
        let pool = ThreadPool::with_workers(2);
        assert!(pool.start().is_ok());
        
        let counter = Arc::new(AtomicUsize::new(0));
        
        // Create a batch of jobs
        let mut batch = JobBatch::new("test_batch");
        
        for i in 0..5 {
            let counter_clone = counter.clone();
            batch.add(CallbackJob::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                let _ = crate::log_info!("Job {} executed", i);
                Ok(())
            }, format!("job_{}", i)));
        }
        
        assert!(pool.submit_batch(batch).is_ok());
        
        // Give time for all jobs to complete
        std::thread::sleep(Duration::from_millis(100));
        
        assert_eq!(counter.load(Ordering::SeqCst), 5);
        
        // Clean up
        pool.stop(true);
    }
    
    #[test]
    fn test_thread_pool_display() {
        let pool = ThreadPool::new("test_pool");
        assert!(pool.to_string().contains("test_pool"));
        assert!(pool.to_string().contains("running: no"));
    }
}