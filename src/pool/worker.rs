//! Worker thread implementation

use crate::core::{BoxedJob, Result, ThreadError};
use crate::queue::{JobQueue, QueueError};
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
    /// Create and start a new worker with a JobQueue
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this worker
    /// * `queue` - Job queue implementing the JobQueue trait
    /// * `poll_interval` - Duration between poll attempts for new jobs
    ///
    /// # Shutdown Behavior
    ///
    /// Workers exit when the queue is closed and empty,
    /// ensuring all queued jobs are processed before shutdown completes.
    pub fn new(
        id: usize,
        queue: Arc<dyn JobQueue>,
        poll_interval: Duration,
    ) -> Result<Self> {
        let stats = Arc::new(WorkerStats::new());
        let stats_clone = Arc::clone(&stats);

        let thread = thread::Builder::new()
            .name(format!("worker-{}", id))
            .spawn(move || {
                Self::run(id, queue, stats_clone, poll_interval);
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
    /// Workers process jobs from the queue until it is closed and empty.
    /// This ensures all queued jobs are processed before shutdown.
    fn run(
        id: usize,
        queue: Arc<dyn JobQueue>,
        stats: Arc<WorkerStats>,
        poll_interval: Duration,
    ) {
        loop {
            match queue.recv_timeout(poll_interval) {
                Ok(mut job) => {
                    Self::execute_job(id, &mut job, &stats);
                }
                Err(QueueError::Empty) => {
                    // No job available within timeout, continue polling
                    continue;
                }
                Err(QueueError::Disconnected) => {
                    // Queue closed and empty, shutdown
                    break;
                }
                Err(_) => {
                    // Other errors, shutdown
                    break;
                }
            }
        }
    }

    /// Execute a single job with panic protection
    fn execute_job(id: usize, job: &mut BoxedJob, stats: &WorkerStats) {
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
    use crate::queue::ChannelQueue;

    #[test]
    fn test_worker_creation() {
        let queue: Arc<dyn JobQueue> = Arc::new(ChannelQueue::unbounded());
        let poll_interval = Duration::from_millis(100);

        let worker =
            Worker::new(0, Arc::clone(&queue), poll_interval).expect("Failed to create worker");
        assert_eq!(worker.id(), 0);

        // Close queue to trigger worker shutdown
        queue.close();
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_worker_job_execution() {
        let queue: Arc<dyn JobQueue> = Arc::new(ChannelQueue::unbounded());
        let poll_interval = Duration::from_millis(100);

        let worker = Worker::new(0, Arc::clone(&queue), poll_interval)
            .expect("Failed to create worker");
        let stats = worker.stats();

        // Send a job
        let job = Box::new(ClosureJob::new(|| Ok(())));
        queue.send(job).expect("Failed to send job");

        // Wait a bit for job to be processed
        thread::sleep(Duration::from_millis(50));

        // Check stats
        assert_eq!(stats.get_jobs_processed(), 1);
        assert_eq!(stats.get_jobs_failed(), 0);

        // Close queue to trigger worker shutdown
        queue.close();
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_worker_panic_handling() {
        let queue: Arc<dyn JobQueue> = Arc::new(ChannelQueue::unbounded());
        let poll_interval = Duration::from_millis(100);

        let worker = Worker::new(0, Arc::clone(&queue), poll_interval)
            .expect("Failed to create worker");
        let stats = worker.stats();

        // Send a job that panics
        let panicking_job = Box::new(ClosureJob::new(|| {
            panic!("Intentional panic for testing");
        }));
        queue
            .send(panicking_job)
            .expect("Failed to send panicking job");

        // Wait for job to be processed
        thread::sleep(Duration::from_millis(100));

        // Check that panic was caught and counted
        assert_eq!(stats.get_jobs_panicked(), 1);
        assert_eq!(stats.get_jobs_processed(), 0);
        assert_eq!(stats.get_jobs_failed(), 0);

        // Send another job to verify worker is still alive
        let normal_job = Box::new(ClosureJob::new(|| Ok(())));
        queue.send(normal_job).expect("Failed to send normal job");

        thread::sleep(Duration::from_millis(50));

        // Verify worker continued processing after panic
        assert_eq!(stats.get_jobs_processed(), 1);
        assert_eq!(stats.get_jobs_panicked(), 1);

        // Close queue to trigger worker shutdown
        queue.close();
        worker.join().expect("Failed to join worker");
    }
}
