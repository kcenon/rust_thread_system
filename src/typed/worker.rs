//! Typed worker implementation for type-aware scheduling.
//!
//! This module provides [`TypedWorker`] which processes jobs from
//! multiple type-specific queues in priority order.

use super::{AtomicTypeStats, JobType};
use crate::core::{BoxedJob, Result, ThreadError};
use crate::queue::{JobQueue, QueueError};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A worker that processes jobs from type-specific queues.
///
/// Workers poll queues in priority order, processing jobs from
/// higher-priority types before lower-priority ones.
pub struct TypedWorker<T: JobType> {
    id: usize,
    job_type: T,
    thread: Option<thread::JoinHandle<()>>,
}

impl<T: JobType> std::fmt::Debug for TypedWorker<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedWorker")
            .field("id", &self.id)
            .field("job_type", &self.job_type)
            .finish()
    }
}

impl<T: JobType> TypedWorker<T> {
    /// Creates and starts a new typed worker.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this worker
    /// * `job_type` - The primary job type this worker handles
    /// * `queue` - The queue for this job type
    /// * `all_queues` - All queues in priority order (for work stealing)
    /// * `stats` - Per-type statistics trackers
    /// * `running` - Shared running flag for shutdown
    /// * `poll_interval` - Duration between poll attempts
    /// * `thread_name_prefix` - Prefix for thread name
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: usize,
        job_type: T,
        queue: Arc<dyn JobQueue>,
        all_queues: Vec<(T, Arc<dyn JobQueue>)>,
        stats: Arc<HashMap<T, Arc<AtomicTypeStats>>>,
        running: Arc<AtomicBool>,
        poll_interval: Duration,
        thread_name_prefix: &str,
    ) -> Result<Self> {
        let thread_name = format!("{}-{}-{}", thread_name_prefix, job_type.name(), id);

        let thread = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                Self::run(
                    id,
                    job_type,
                    queue,
                    all_queues,
                    stats,
                    running,
                    poll_interval,
                );
            })
            .map_err(|e| ThreadError::spawn(id, e.to_string()))?;

        Ok(Self {
            id,
            job_type,
            thread: Some(thread),
        })
    }

    /// Returns the worker ID.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Returns the job type this worker handles.
    pub fn job_type(&self) -> T {
        self.job_type
    }

    /// Joins the worker thread.
    pub fn join(mut self) -> Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| ThreadError::join(self.id, "Worker panicked"))?;
        }
        Ok(())
    }

    /// Main worker loop.
    fn run(
        id: usize,
        primary_type: T,
        primary_queue: Arc<dyn JobQueue>,
        all_queues: Vec<(T, Arc<dyn JobQueue>)>,
        stats: Arc<HashMap<T, Arc<AtomicTypeStats>>>,
        running: Arc<AtomicBool>,
        poll_interval: Duration,
    ) {
        loop {
            // First, try to get a job from our primary queue
            match primary_queue.recv_timeout(poll_interval) {
                Ok(mut job) => {
                    if let Some(stat) = stats.get(&primary_type) {
                        Self::execute_job(id, &mut job, stat);
                    }
                    continue;
                }
                Err(QueueError::Empty) => {
                    // No job in primary queue, try work stealing
                }
                Err(QueueError::Disconnected) => {
                    // Queue closed and empty, exit
                    break;
                }
                Err(_) => {
                    // Other errors
                    if !running.load(Ordering::Acquire) {
                        break;
                    }
                    continue;
                }
            }

            // Try to steal work from other queues in priority order
            let mut job_found = false;
            for (job_type, queue) in &all_queues {
                if *job_type == primary_type {
                    continue; // Skip our primary queue, already checked
                }

                if let Ok(mut job) = queue.try_recv() {
                    if let Some(stat) = stats.get(job_type) {
                        Self::execute_job(id, &mut job, stat);
                    }
                    job_found = true;
                    break;
                }
            }

            if !job_found {
                // No jobs available anywhere, check if should stop
                if !running.load(Ordering::Acquire) {
                    // Drain remaining jobs from primary queue before exiting
                    while let Ok(mut job) = primary_queue.try_recv() {
                        if let Some(stat) = stats.get(&primary_type) {
                            Self::execute_job(id, &mut job, stat);
                        }
                    }
                    break;
                }
            }
        }
    }

    /// Execute a single job with panic protection.
    fn execute_job(id: usize, job: &mut BoxedJob, stats: &AtomicTypeStats) {
        let start = std::time::Instant::now();

        let panic_result = catch_unwind(AssertUnwindSafe(|| job.execute()));

        let elapsed = start.elapsed();

        match panic_result {
            Ok(Ok(())) => {
                stats.record_completion(elapsed);
            }
            Ok(Err(e)) => {
                eprintln!("TypedWorker {}: Job execution failed: {}", id, e);
                stats.record_failure(elapsed);
            }
            Err(panic_info) => {
                let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                eprintln!("TypedWorker {}: Job panicked: {}", id, panic_msg);
                stats.record_panic(elapsed);
            }
        }
    }
}

impl<T: JobType> Drop for TypedWorker<T> {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            const JOIN_TIMEOUT: Duration = Duration::from_secs(5);
            let start = std::time::Instant::now();

            loop {
                if thread.is_finished() {
                    match thread.join() {
                        Ok(()) => break,
                        Err(panic_info) => {
                            let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "Unknown panic".to_string()
                            };
                            eprintln!(
                                "[TYPED_WORKER ERROR] Worker {} panicked during shutdown: {}",
                                self.id, panic_msg
                            );
                            break;
                        }
                    }
                }

                if start.elapsed() >= JOIN_TIMEOUT {
                    eprintln!(
                        "[TYPED_WORKER WARNING] Worker {} did not finish within {}s timeout during drop.",
                        self.id,
                        JOIN_TIMEOUT.as_secs()
                    );
                    break;
                }

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
    use crate::typed::DefaultJobType;

    fn create_test_setup() -> (
        Arc<dyn JobQueue>,
        Vec<(DefaultJobType, Arc<dyn JobQueue>)>,
        Arc<HashMap<DefaultJobType, Arc<AtomicTypeStats>>>,
        Arc<AtomicBool>,
    ) {
        let queue: Arc<dyn JobQueue> = Arc::new(ChannelQueue::unbounded());
        let all_queues = vec![(DefaultJobType::Compute, Arc::clone(&queue))];

        let mut stats_map = HashMap::new();
        stats_map.insert(DefaultJobType::Compute, Arc::new(AtomicTypeStats::new()));
        let stats = Arc::new(stats_map);

        let running = Arc::new(AtomicBool::new(true));

        (queue, all_queues, stats, running)
    }

    #[test]
    fn test_typed_worker_creation() {
        let (queue, all_queues, stats, running) = create_test_setup();

        let worker = TypedWorker::new(
            0,
            DefaultJobType::Compute,
            queue.clone(),
            all_queues,
            stats,
            running.clone(),
            Duration::from_millis(50),
            "test-worker",
        )
        .expect("Failed to create worker");

        assert_eq!(worker.id(), 0);
        assert_eq!(worker.job_type(), DefaultJobType::Compute);

        running.store(false, Ordering::Release);
        queue.close();
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_typed_worker_job_execution() {
        let (queue, all_queues, stats, running) = create_test_setup();

        let worker = TypedWorker::new(
            0,
            DefaultJobType::Compute,
            queue.clone(),
            all_queues,
            stats.clone(),
            running.clone(),
            Duration::from_millis(50),
            "test-worker",
        )
        .expect("Failed to create worker");

        // Send a job
        let job = Box::new(ClosureJob::new(|| Ok(())));
        queue.send(job).expect("Failed to send job");

        // Wait for job to be processed
        thread::sleep(Duration::from_millis(100));

        // Check stats
        let type_stats = stats.get(&DefaultJobType::Compute).unwrap();
        assert_eq!(type_stats.jobs_completed(), 1);

        running.store(false, Ordering::Release);
        queue.close();
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_typed_worker_handles_failures() {
        let (queue, all_queues, stats, running) = create_test_setup();

        let worker = TypedWorker::new(
            0,
            DefaultJobType::Compute,
            queue.clone(),
            all_queues,
            stats.clone(),
            running.clone(),
            Duration::from_millis(50),
            "test-worker",
        )
        .expect("Failed to create worker");

        // Send a failing job
        let job = Box::new(ClosureJob::new(|| {
            Err(ThreadError::other("Intentional failure"))
        }));
        queue.send(job).expect("Failed to send job");

        thread::sleep(Duration::from_millis(100));

        let type_stats = stats.get(&DefaultJobType::Compute).unwrap();
        assert_eq!(type_stats.jobs_failed(), 1);

        running.store(false, Ordering::Release);
        queue.close();
        worker.join().expect("Failed to join worker");
    }

    #[test]
    fn test_typed_worker_handles_panics() {
        let (queue, all_queues, stats, running) = create_test_setup();

        let worker = TypedWorker::new(
            0,
            DefaultJobType::Compute,
            queue.clone(),
            all_queues,
            stats.clone(),
            running.clone(),
            Duration::from_millis(50),
            "test-worker",
        )
        .expect("Failed to create worker");

        // Send a panicking job
        let job = Box::new(ClosureJob::new(|| {
            panic!("Intentional panic");
        }));
        queue.send(job).expect("Failed to send job");

        thread::sleep(Duration::from_millis(100));

        let type_stats = stats.get(&DefaultJobType::Compute).unwrap();
        assert_eq!(type_stats.jobs_panicked(), 1);

        // Verify worker is still alive
        let job2 = Box::new(ClosureJob::new(|| Ok(())));
        queue.send(job2).expect("Failed to send second job");

        thread::sleep(Duration::from_millis(100));
        assert_eq!(type_stats.jobs_completed(), 1);

        running.store(false, Ordering::Release);
        queue.close();
        worker.join().expect("Failed to join worker");
    }
}
