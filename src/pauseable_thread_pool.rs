//! Pauseable thread pool implementation for managing worker threads.
//!
//! This module provides a thread pool implementation that can be paused and resumed,
//! which temporarily suspends processing jobs without stopping worker threads.

use std::fmt;
use std::sync::{Arc, Mutex, Condvar, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::job::{Job, JobQueue, BasicJobQueue, JobBatch};
use crate::thread_base::{ThreadWorker, ThreadCondition};

/// A pauseable thread worker that can be temporarily suspended.
pub struct PauseableThreadWorker {
    /// Title for the thread
    title: String,
    
    /// Flag indicating if the thread is running
    running: Arc<AtomicBool>,
    
    /// Flag indicating if the thread is paused
    paused: Arc<AtomicBool>,
    
    /// Conditional variable for pausing and resuming
    pause_condvar: Arc<(Mutex<bool>, Condvar)>,
    
    /// Handle to the thread, if started
    handle: Option<std::thread::JoinHandle<()>>,
    
    /// Job queue to pull work from
    job_queue: Arc<dyn JobQueue>,
    
    /// Interval to check for new jobs when idle
    check_interval: Duration,
    
    /// Current condition of the thread
    condition: ThreadCondition,
}

impl PauseableThreadWorker {
    /// Create a new pauseable thread worker with the given job queue.
    pub fn new(
        title: impl Into<String>,
        job_queue: Arc<dyn JobQueue>,
        check_interval: Duration,
        paused: Arc<AtomicBool>,
        pause_condvar: Arc<(Mutex<bool>, Condvar)>,
    ) -> Self {
        Self {
            title: title.into(),
            running: Arc::new(AtomicBool::new(false)),
            paused,
            pause_condvar,
            handle: None,
            job_queue,
            check_interval,
            condition: ThreadCondition::NotStarted,
        }
    }
    
    /// Get the current thread condition
    pub fn condition(&self) -> ThreadCondition {
        self.condition
    }
}

impl ThreadWorker for PauseableThreadWorker {
    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Err(Error::thread_error(
                format!("Worker '{}' is already running", self.title)
            ));
        }
        
        self.running.store(true, Ordering::SeqCst);
        self.condition = ThreadCondition::Running;
        
        let running = self.running.clone();
        let paused = self.paused.clone();
        let job_queue = self.job_queue.clone();
        let thread_title_for_thread = self.title.clone();
        let thread_title_for_error = self.title.clone();
        let check_interval = self.check_interval;
        let pause_condvar = self.pause_condvar.clone();
        
        let handle = std::thread::Builder::new()
            .name(thread_title_for_thread.clone())
            .spawn(move || {
                while running.load(Ordering::SeqCst) {
                    // Check if the worker is paused
                    if paused.load(Ordering::SeqCst) {
                        // Wait until not paused
                        let (lock, cvar) = &*pause_condvar;
                        let mut paused_status = lock.lock().unwrap_or_else(|e| {
                            let _ = crate::log_error!("Lock error in worker '{}': {:?}", thread_title_for_thread, e);
                            e.into_inner()
                        });
                        while paused.load(Ordering::SeqCst) && running.load(Ordering::SeqCst) {
                            paused_status = cvar.wait(paused_status).unwrap_or_else(|e| {
                                let _ = crate::log_error!("Wait error in worker '{}': {:?}", thread_title_for_thread, e);
                                e.into_inner()
                            });
                        }
                        
                        // If we were signaled to stop while paused, exit now
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                    
                    // Try to get a job from the queue
                    if let Some(job) = job_queue.dequeue() {
                        if let Err(e) = job.execute() {
                            let _ = crate::log_error!("Job execution error in worker '{}': {}", thread_title_for_thread, e);
                        }
                    } else {
                        // No job available, sleep for a bit
                        std::thread::sleep(check_interval);
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
        
        // Wake up paused threads to allow them to exit
        if self.paused.load(Ordering::SeqCst) {
            let (_, cvar) = &*self.pause_condvar;
            cvar.notify_all();
        }
        
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
        format!("PauseableThreadWorker '{}' ({}, {})",
            self.title,
            if self.is_running() { "running" } else { "stopped" },
            if self.paused.load(Ordering::SeqCst) { "paused" } else { "active" }
        )
    }
}

impl fmt::Debug for PauseableThreadWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PauseableThreadWorker")
            .field("title", &self.title)
            .field("running", &self.is_running())
            .field("paused", &self.paused.load(Ordering::SeqCst))
            .field("condition", &self.condition)
            .finish()
    }
}

impl Drop for PauseableThreadWorker {
    fn drop(&mut self) {
        self.stop(true);
    }
}

/// Thread pool that manages pauseable worker threads and distributes jobs.
pub struct PauseableThreadPool {
    /// Title for the thread pool
    title: String,
    
    /// Flag indicating if the pool is running
    running: Arc<AtomicBool>,
    
    /// Flag indicating if the pool is paused
    paused: Arc<AtomicBool>,
    
    /// Conditional variable for pausing and resuming
    pause_condvar: Arc<(Mutex<bool>, Condvar)>,
    
    /// Job queue used by all workers in the pool
    job_queue: Arc<dyn JobQueue>,
    
    /// Worker threads
    workers: Mutex<Vec<Box<dyn ThreadWorker>>>,
    
    /// Default check interval for worker threads
    check_interval: Duration,
}

// We need to implement Clone for pauseable thread pools
impl Clone for PauseableThreadPool {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            running: self.running.clone(),
            paused: self.paused.clone(),
            pause_condvar: self.pause_condvar.clone(),
            job_queue: self.job_queue.clone(),
            workers: Mutex::new(Vec::new()), // Don't clone workers
            check_interval: self.check_interval,
        }
    }
}

impl PauseableThreadPool {
    /// Create a new pauseable thread pool with the given title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            pause_condvar: Arc::new((Mutex::new(false), Condvar::new())),
            job_queue: Arc::new(BasicJobQueue::new()),
            workers: Mutex::new(Vec::new()),
            check_interval: Duration::from_millis(50),
        }
    }
    
    /// Create a new pauseable thread pool with the specified number of worker threads.
    pub fn with_workers(num_workers: usize) -> Self {
        let pool = Self::new("pauseable_thread_pool");
        let _ = pool.add_workers(num_workers);
        pool
    }
    
    /// Add a specified number of worker threads to the pool.
    pub fn add_workers(&self, count: usize) -> Result<()> {
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        for i in 0..count {
            let worker_title = format!("{}_worker_{}", self.title, workers.len() + i);
            let mut worker = Box::new(PauseableThreadWorker::new(
                worker_title,
                self.job_queue.clone(),
                self.check_interval,
                self.paused.clone(),
                self.pause_condvar.clone(),
            ));
            
            if self.is_running() {
                worker.start()?;
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
            worker.start()?;
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
        
        let mut workers = self.workers.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock workers mutex: {}", e))
        )?;
        
        for worker in workers.iter_mut() {
            worker.start()?;
        }
        
        Ok(())
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
            self.job_queue.enqueue(job)?;
        }
        
        Ok(())
    }
    
    /// Stop the thread pool and all worker threads.
    pub fn stop(&self, wait_for_completion: bool) {
        let _ = crate::log_info!("Stopping thread pool '{}'...", self.title);
        
        // First set the running flag to false
        self.running.store(false, Ordering::SeqCst);
        
        // Wake up all paused workers
        if self.paused.load(Ordering::SeqCst) {
            let (_, cvar) = &*self.pause_condvar;
            cvar.notify_all();
        }
        
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
    
    /// Pause the thread pool to temporarily suspend processing jobs.
    pub fn pause(&self) -> Result<()> {
        if !self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Cannot pause thread pool '{}' as it is not running", self.title)
            ));
        }
        
        if self.is_paused() {
            return Ok(());  // Already paused, do nothing
        }
        
        let _ = crate::log_info!("Pausing thread pool '{}'...", self.title);
        
        // Set the paused flag to true
        self.paused.store(true, Ordering::SeqCst);
        
        Ok(())
    }
    
    /// Resume a paused thread pool to continue processing jobs.
    pub fn resume(&self) -> Result<()> {
        if !self.is_running() {
            return Err(Error::thread_pool_error(
                format!("Cannot resume thread pool '{}' as it is not running", self.title)
            ));
        }
        
        if !self.is_paused() {
            return Ok(());  // Not paused, do nothing
        }
        
        let _ = crate::log_info!("Resuming thread pool '{}'...", self.title);
        
        // Set the paused flag to false
        self.paused.store(false, Ordering::SeqCst);
        
        // Notify all waiting threads to wake up
        let (_, cvar) = &*self.pause_condvar;
        cvar.notify_all();
        
        Ok(())
    }
    
    /// Check if the thread pool is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    /// Check if the thread pool is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
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
}

impl fmt::Debug for PauseableThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PauseableThreadPool")
            .field("title", &self.title)
            .field("running", &self.is_running())
            .field("paused", &self.is_paused())
            .field("worker_count", &self.worker_count())
            .finish()
    }
}

impl fmt::Display for PauseableThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PauseableThreadPool '{}' (running: {}, paused: {}, workers: {})",
            self.title,
            if self.is_running() { "yes" } else { "no" },
            if self.is_paused() { "yes" } else { "no" },
            self.worker_count()
        )
    }
}

impl Drop for PauseableThreadPool {
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
    use std::thread;
    
    #[test]
    fn test_pauseable_thread_pool_creation() {
        let pool = PauseableThreadPool::new("test_pool");
        assert_eq!(pool.worker_count(), 0);
        assert!(!pool.is_running());
        assert!(!pool.is_paused());
        
        let pool = PauseableThreadPool::with_workers(4);
        assert_eq!(pool.worker_count(), 4);
    }
    
    #[test]
    fn test_pauseable_thread_pool_start_stop() {
        let pool = PauseableThreadPool::with_workers(2);
        
        assert!(!pool.is_running());
        assert!(pool.start().is_ok());
        assert!(pool.is_running());
        
        // Starting again should fail
        assert!(pool.start().is_err());
        
        pool.stop(true);
        assert!(!pool.is_running());
    }
    
    #[test]
    fn test_pauseable_thread_pool_pause_resume() {
        let pool = PauseableThreadPool::with_workers(2);
        assert!(pool.start().is_ok());
        
        // Initially not paused
        assert!(!pool.is_paused());
        
        // Pause the pool
        assert!(pool.pause().is_ok());
        assert!(pool.is_paused());
        
        // Resume the pool
        assert!(pool.resume().is_ok());
        assert!(!pool.is_paused());
        
        // Clean up
        pool.stop(true);
    }
    
    #[test]
    fn test_pauseable_thread_pool_job_execution() {
        let pool = PauseableThreadPool::with_workers(2);
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
        thread::sleep(Duration::from_millis(50));
        
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        
        // Clean up
        pool.stop(true);
    }
    
    #[test]
    fn test_pauseable_thread_pool_pause_functionality() {
        let pool = PauseableThreadPool::with_workers(2);
        assert!(pool.start().is_ok());
        
        let counter = Arc::new(AtomicUsize::new(0));
        
        // Pause the pool
        assert!(pool.pause().is_ok());
        assert!(pool.is_paused());
        
        // Submit a job while paused
        let counter_clone = counter.clone();
        let job = CallbackJob::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }, "test_job_paused");
        
        assert!(pool.submit(job).is_ok());
        
        // Wait to ensure the job doesn't execute while paused
        thread::sleep(Duration::from_millis(50));
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Job should not execute while pool is paused");
        
        // Resume the pool
        assert!(pool.resume().is_ok());
        
        // Wait for the job to execute after resuming
        thread::sleep(Duration::from_millis(50));
        assert_eq!(counter.load(Ordering::SeqCst), 1, "Job should execute after pool is resumed");
        
        // Clean up
        pool.stop(true);
    }
    
    #[test]
    fn test_pauseable_thread_pool_batch_submission() {
        let pool = PauseableThreadPool::with_workers(2);
        assert!(pool.start().is_ok());
        
        let counter = Arc::new(AtomicUsize::new(0));
        
        // Create a batch of jobs
        let mut batch = JobBatch::new("test_batch");
        
        for i in 0..5 {
            let counter_clone = counter.clone();
            batch.add(CallbackJob::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }, format!("job_{}", i)));
        }
        
        // Pause the pool
        assert!(pool.pause().is_ok());
        
        // Submit the batch while paused
        assert!(pool.submit_batch(batch).is_ok());
        
        // Wait to ensure jobs don't execute while paused
        thread::sleep(Duration::from_millis(50));
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Jobs should not execute while pool is paused");
        
        // Resume the pool
        assert!(pool.resume().is_ok());
        
        // Wait for all jobs to complete after resuming
        thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::SeqCst), 5, "All jobs should execute after pool is resumed");
        
        // Clean up
        pool.stop(true);
    }
    
    #[test]
    fn test_pauseable_thread_pool_display() {
        let pool = PauseableThreadPool::new("test_pool");
        assert!(pool.to_string().contains("test_pool"));
        assert!(pool.to_string().contains("running: no"));
        assert!(pool.to_string().contains("paused: no"));
    }
}