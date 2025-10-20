//! Worker thread implementation

use crate::core::{BoxedJob, Result, ThreadError};
use crossbeam::channel::{Receiver, RecvTimeoutError};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Statistics for a worker thread
#[derive(Debug, Default)]
pub struct WorkerStats {
    /// Total number of jobs processed
    pub jobs_processed: AtomicU64,
    /// Total number of jobs that failed
    pub jobs_failed: AtomicU64,
    /// Total number of jobs that panicked
    pub jobs_panicked: AtomicU64,
    /// Total time spent processing jobs (microseconds)
    pub total_processing_time_us: AtomicU64,
}

impl WorkerStats {
    /// Create new worker statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment jobs processed counter
    pub fn increment_processed(&self) {
        self.jobs_processed.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment jobs failed counter
    pub fn increment_failed(&self) {
        self.jobs_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment jobs panicked counter
    pub fn increment_panicked(&self) {
        self.jobs_panicked.fetch_add(1, Ordering::Relaxed);
    }

    /// Add processing time
    pub fn add_processing_time(&self, microseconds: u64) {
        self.total_processing_time_us
            .fetch_add(microseconds, Ordering::Relaxed);
    }

    /// Get total jobs processed
    pub fn get_jobs_processed(&self) -> u64 {
        self.jobs_processed.load(Ordering::Relaxed)
    }

    /// Get total jobs failed
    pub fn get_jobs_failed(&self) -> u64 {
        self.jobs_failed.load(Ordering::Relaxed)
    }

    /// Get total jobs panicked
    pub fn get_jobs_panicked(&self) -> u64 {
        self.jobs_panicked.load(Ordering::Relaxed)
    }

    /// Get average processing time per job in microseconds
    pub fn get_average_processing_time_us(&self) -> f64 {
        let total = self.total_processing_time_us.load(Ordering::Relaxed);
        let count = self.jobs_processed.load(Ordering::Relaxed);
        if count > 0 {
            total as f64 / count as f64
        } else {
            0.0
        }
    }
}

/// A worker thread that processes jobs from a queue
#[derive(Debug)]
pub struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
    stats: Arc<WorkerStats>,
}

impl Worker {
    /// Create and start a new worker
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this worker
    /// * `receiver` - Channel receiver for receiving jobs
    /// * `queue_size` - Shared counter for tracking queue size
    ///
    /// # Shutdown Behavior
    ///
    /// Workers exit when the channel is disconnected (sender is dropped),
    /// NOT via a shutdown flag. This ensures all queued jobs are processed
    /// before shutdown completes.
    pub fn new(
        id: usize,
        receiver: Receiver<BoxedJob>,
        queue_size: Arc<AtomicU64>,
    ) -> Result<Self> {
        let stats = Arc::new(WorkerStats::new());
        let stats_clone = Arc::clone(&stats);

        let thread = thread::Builder::new()
            .name(format!("worker-{}", id))
            .spawn(move || {
                Self::run(id, receiver, stats_clone, queue_size);
            })
            .map_err(|e| ThreadError::spawn(id, e.to_string()))?;

        Ok(Self {
            id,
            thread: Some(thread),
            stats,
        })
    }

    /// Get worker ID
    pub fn id(&self) -> usize {
        self.id
    }

    /// Get worker statistics
    pub fn stats(&self) -> Arc<WorkerStats> {
        Arc::clone(&self.stats)
    }

    /// Join the worker thread
    pub fn join(mut self) -> Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| ThreadError::join(self.id, "Worker panicked"))?;
        }
        Ok(())
    }

    /// Main worker loop
    ///
    /// Workers process jobs from the receiver until the channel is disconnected.
    /// This ensures all queued jobs are processed before shutdown.
    fn run(
        id: usize,
        receiver: Receiver<BoxedJob>,
        stats: Arc<WorkerStats>,
        queue_size: Arc<AtomicU64>,
    ) {
        loop {
            // Try to receive a job with timeout
            // Workers exit when channel is disconnected (RecvTimeoutError::Disconnected)
            // This ensures all queued jobs are drained before shutdown completes
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(mut job) => {
                    // Decrement queue size as we're processing this job (with underflow protection)
                    queue_size
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |size| {
                            if size > 0 {
                                Some(size - 1)
                            } else {
                                Some(0)
                            }
                        })
                        .ok();

                    let start = std::time::Instant::now();

                    // Execute the job with panic protection
                    let panic_result = catch_unwind(AssertUnwindSafe(|| job.execute()));

                    match panic_result {
                        Ok(Ok(())) => {
                            // Job completed successfully
                            stats.increment_processed();
                        }
                        Ok(Err(e)) => {
                            // Job returned an error
                            eprintln!("Worker {}: Job execution failed: {}", id, e);
                            stats.increment_failed();
                        }
                        Err(panic_info) => {
                            // Job panicked
                            let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "Unknown panic".to_string()
                            };
                            eprintln!("Worker {}: Job panicked: {}", id, panic_msg);
                            stats.increment_panicked();
                        }
                    }

                    let elapsed = start.elapsed().as_micros() as u64;
                    stats.add_processing_time(elapsed);
                }
                Err(RecvTimeoutError::Timeout) => {
                    // No job available, continue
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // Channel closed, shutdown
                    break;
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // Use a timeout to prevent Drop from hanging indefinitely
            const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

            let start = std::time::Instant::now();
            loop {
                if thread.is_finished() {
                    // Thread finished, join to check for panics
                    match thread.join() {
                        Ok(()) => {
                            // Clean shutdown
                            break;
                        }
                        Err(panic_info) => {
                            // Worker panicked during shutdown
                            let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "Unknown panic".to_string()
                            };
                            eprintln!(
                                "[WORKER ERROR] Worker {} panicked during shutdown: {}",
                                self.id, panic_msg
                            );
                            break;
                        }
                    }
                }

                if start.elapsed() >= JOIN_TIMEOUT {
                    eprintln!(
                        "[WORKER WARNING] Worker {} did not finish within {}s timeout during drop. \
                         Thread may be leaked.",
                        self.id,
                        JOIN_TIMEOUT.as_secs()
                    );
                    break;
                }

                // Small sleep to avoid busy-waiting
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ClosureJob;
    use crossbeam::channel::unbounded;

    #[test]
    fn test_worker_creation() {
        let (sender, receiver) = unbounded();
        let queue_size = Arc::new(AtomicU64::new(0));

        let worker = Worker::new(0, receiver, queue_size).expect("Failed to create worker");
        assert_eq!(worker.id(), 0);

        // Disconnect channel to trigger worker shutdown
        drop(sender);
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_worker_job_execution() {
        let (sender, receiver) = unbounded();
        let queue_size = Arc::new(AtomicU64::new(0));

        let worker =
            Worker::new(0, receiver, Arc::clone(&queue_size)).expect("Failed to create worker");
        let stats = worker.stats();

        // Send a job
        queue_size.fetch_add(1, Ordering::Relaxed);
        let job = Box::new(ClosureJob::new(|| Ok(())));
        sender.send(job).expect("Failed to send job");

        // Wait a bit for job to be processed
        thread::sleep(Duration::from_millis(50));

        // Check stats
        assert_eq!(stats.get_jobs_processed(), 1);
        assert_eq!(stats.get_jobs_failed(), 0);
        assert_eq!(queue_size.load(Ordering::Relaxed), 0);

        // Disconnect channel to trigger worker shutdown
        drop(sender);
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_worker_panic_handling() {
        let (sender, receiver) = unbounded();
        let queue_size = Arc::new(AtomicU64::new(0));

        let worker =
            Worker::new(0, receiver, Arc::clone(&queue_size)).expect("Failed to create worker");
        let stats = worker.stats();

        // Send a job that panics
        queue_size.fetch_add(1, Ordering::Relaxed);
        let panicking_job = Box::new(ClosureJob::new(|| {
            panic!("Intentional panic for testing");
        }));
        sender
            .send(panicking_job)
            .expect("Failed to send panicking job");

        // Wait for job to be processed
        thread::sleep(Duration::from_millis(100));

        // Check that panic was caught and counted
        assert_eq!(stats.get_jobs_panicked(), 1);
        assert_eq!(stats.get_jobs_processed(), 0);
        assert_eq!(stats.get_jobs_failed(), 0);

        // Send another job to verify worker is still alive
        queue_size.fetch_add(1, Ordering::Relaxed);
        let normal_job = Box::new(ClosureJob::new(|| Ok(())));
        sender.send(normal_job).expect("Failed to send normal job");

        thread::sleep(Duration::from_millis(50));

        // Verify worker continued processing after panic
        assert_eq!(stats.get_jobs_processed(), 1);
        assert_eq!(stats.get_jobs_panicked(), 1);

        // Disconnect channel to trigger worker shutdown
        drop(sender);
        worker.join().expect("Failed to join worker");
    }
}
