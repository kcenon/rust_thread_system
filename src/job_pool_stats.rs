//! Thread pool job statistics tracking.
//!
//! This module provides statistics collection functionality for thread pools,
//! tracking metrics such as job counts, execution times, and success/failure rates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::priority_thread_pool::JobPriority;

/// Statistics tracker for thread pool job execution.
///
/// Provides thread-safe tracking of job execution statistics including counts, durations,
/// success rates, and priority-based metrics.
#[derive(Debug)]
pub struct JobPoolStats {
    /// Total number of jobs submitted to the pool
    jobs_submitted: AtomicUsize,
    
    /// Total number of jobs that completed successfully
    jobs_succeeded: AtomicUsize,
    
    /// Total number of jobs that failed
    jobs_failed: AtomicUsize,
    
    /// Current number of jobs in the queue
    jobs_queued: AtomicUsize,
    
    /// Sum of all job execution times in microseconds
    total_execution_time_us: AtomicU64,
    
    /// Minimum job execution time in microseconds
    min_execution_time_us: AtomicU64,
    
    /// Maximum job execution time in microseconds
    max_execution_time_us: AtomicU64,
    
    /// Number of jobs tracked for average calculation
    jobs_timed: AtomicUsize,
    
    /// Number of jobs completed per priority level
    jobs_by_priority: RwLock<HashMap<JobPriority, usize>>,
    
    /// Creation time of the stats tracker
    creation_time: Instant,
}

impl JobPoolStats {
    /// Create a new job pool statistics tracker.
    pub fn new() -> Self {
        Self {
            jobs_submitted: AtomicUsize::new(0),
            jobs_succeeded: AtomicUsize::new(0),
            jobs_failed: AtomicUsize::new(0),
            jobs_queued: AtomicUsize::new(0),
            total_execution_time_us: AtomicU64::new(0),
            min_execution_time_us: AtomicU64::new(u64::MAX),
            max_execution_time_us: AtomicU64::new(0),
            jobs_timed: AtomicUsize::new(0),
            jobs_by_priority: RwLock::new(HashMap::new()),
            creation_time: Instant::now(),
        }
    }

    /// Create a shared instance of job pool statistics.
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
    
    /// Register a job submission to the pool.
    pub fn register_job_submission(&self) {
        self.jobs_submitted.fetch_add(1, Ordering::SeqCst);
        self.jobs_queued.fetch_add(1, Ordering::SeqCst);
    }
    
    /// Register multiple job submissions to the pool.
    pub fn register_job_submissions(&self, count: usize) {
        self.jobs_submitted.fetch_add(count, Ordering::SeqCst);
        self.jobs_queued.fetch_add(count, Ordering::SeqCst);
    }
    
    /// Register a job completion with the specified duration.
    pub fn register_job_completed(
        &self,
        succeeded: bool,
        execution_time: Duration,
        priority: Option<JobPriority>,
    ) {
        // Update success/failure counts
        if succeeded {
            self.jobs_succeeded.fetch_add(1, Ordering::SeqCst);
        } else {
            self.jobs_failed.fetch_add(1, Ordering::SeqCst);
        }
        
        // Decrement queue count
        self.jobs_queued.fetch_sub(1, Ordering::SeqCst);
        
        // Update timing statistics
        let execution_time_us = execution_time.as_micros() as u64;
        
        self.total_execution_time_us.fetch_add(execution_time_us, Ordering::SeqCst);
        self.jobs_timed.fetch_add(1, Ordering::SeqCst);
        
        // Update min time (using atomic compare-and-swap pattern)
        let mut current_min = self.min_execution_time_us.load(Ordering::Relaxed);
        while execution_time_us < current_min {
            match self.min_execution_time_us.compare_exchange(
                current_min,
                execution_time_us,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }
        
        // Update max time (using atomic compare-and-swap pattern)
        let mut current_max = self.max_execution_time_us.load(Ordering::Relaxed);
        while execution_time_us > current_max {
            match self.max_execution_time_us.compare_exchange(
                current_max,
                execution_time_us,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
        
        // Update priority statistics if provided
        if let Some(priority) = priority {
            if let Ok(mut priorities) = self.jobs_by_priority.write() {
                *priorities.entry(priority).or_insert(0) += 1;
            }
        }
    }
    
    /// Get the total number of jobs submitted to the pool.
    pub fn jobs_submitted(&self) -> usize {
        self.jobs_submitted.load(Ordering::SeqCst)
    }
    
    /// Get the total number of jobs that completed successfully.
    pub fn jobs_succeeded(&self) -> usize {
        self.jobs_succeeded.load(Ordering::SeqCst)
    }
    
    /// Get the total number of jobs that failed.
    pub fn jobs_failed(&self) -> usize {
        self.jobs_failed.load(Ordering::SeqCst)
    }
    
    /// Get the current number of jobs in the queue.
    pub fn jobs_queued(&self) -> usize {
        self.jobs_queued.load(Ordering::SeqCst)
    }
    
    /// Get the average job execution time.
    pub fn average_execution_time(&self) -> Option<Duration> {
        let jobs_timed = self.jobs_timed.load(Ordering::SeqCst);
        if jobs_timed == 0 {
            return None;
        }
        
        let total_us = self.total_execution_time_us.load(Ordering::SeqCst);
        let avg_us = total_us / jobs_timed as u64;
        
        Some(Duration::from_micros(avg_us))
    }
    
    /// Get the minimum job execution time.
    pub fn min_execution_time(&self) -> Option<Duration> {
        let min_us = self.min_execution_time_us.load(Ordering::SeqCst);
        if min_us == u64::MAX {
            return None;
        }
        
        Some(Duration::from_micros(min_us))
    }
    
    /// Get the maximum job execution time.
    pub fn max_execution_time(&self) -> Option<Duration> {
        let max_us = self.max_execution_time_us.load(Ordering::SeqCst);
        if max_us == 0 {
            return None;
        }
        
        Some(Duration::from_micros(max_us))
    }
    
    /// Get the number of completed jobs by priority.
    pub fn jobs_by_priority(&self) -> HashMap<JobPriority, usize> {
        self.jobs_by_priority.read().map(|p| p.clone()).unwrap_or_default()
    }
    
    /// Get the total completion rate (successful / total).
    pub fn completion_rate(&self) -> f64 {
        let total = self.jobs_submitted.load(Ordering::SeqCst);
        if total == 0 {
            return 0.0;
        }
        
        let succeeded = self.jobs_succeeded.load(Ordering::SeqCst);
        succeeded as f64 / total as f64
    }
    
    /// Get the uptime of the stats tracker.
    pub fn uptime(&self) -> Duration {
        self.creation_time.elapsed()
    }
    
    /// Reset all statistics to initial values.
    pub fn reset(&self) {
        self.jobs_submitted.store(0, Ordering::SeqCst);
        self.jobs_succeeded.store(0, Ordering::SeqCst);
        self.jobs_failed.store(0, Ordering::SeqCst);
        self.jobs_queued.store(0, Ordering::SeqCst);
        self.total_execution_time_us.store(0, Ordering::SeqCst);
        self.min_execution_time_us.store(u64::MAX, Ordering::SeqCst);
        self.max_execution_time_us.store(0, Ordering::SeqCst);
        self.jobs_timed.store(0, Ordering::SeqCst);
        
        if let Ok(mut priorities) = self.jobs_by_priority.write() {
            priorities.clear();
        }
    }
}

impl Default for JobPoolStats {
    fn default() -> Self {
        Self::new()
    }
}

/// A value object representing a snapshot of job pool statistics.
#[derive(Debug, Clone)]
pub struct JobPoolStatsSnapshot {
    /// Total number of jobs submitted to the pool
    pub jobs_submitted: usize,
    
    /// Total number of jobs that completed successfully
    pub jobs_succeeded: usize,
    
    /// Total number of jobs that failed
    pub jobs_failed: usize,
    
    /// Current number of jobs in the queue
    pub jobs_queued: usize,
    
    /// Average job execution time
    pub average_execution_time: Option<Duration>,
    
    /// Minimum job execution time
    pub min_execution_time: Option<Duration>,
    
    /// Maximum job execution time
    pub max_execution_time: Option<Duration>,
    
    /// Number of completed jobs by priority
    pub jobs_by_priority: HashMap<JobPriority, usize>,
    
    /// Completion rate (successful / total)
    pub completion_rate: f64,
    
    /// Uptime of the stats tracker
    pub uptime: Duration,
}

impl JobPoolStatsSnapshot {
    /// Create a new snapshot from the given job pool stats.
    pub fn from_stats(stats: &JobPoolStats) -> Self {
        Self {
            jobs_submitted: stats.jobs_submitted(),
            jobs_succeeded: stats.jobs_succeeded(),
            jobs_failed: stats.jobs_failed(),
            jobs_queued: stats.jobs_queued(),
            average_execution_time: stats.average_execution_time(),
            min_execution_time: stats.min_execution_time(),
            max_execution_time: stats.max_execution_time(),
            jobs_by_priority: stats.jobs_by_priority(),
            completion_rate: stats.completion_rate(),
            uptime: stats.uptime(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    
    #[test]
    fn test_new_stats_empty() {
        let stats = JobPoolStats::new();
        
        assert_eq!(stats.jobs_submitted(), 0);
        assert_eq!(stats.jobs_succeeded(), 0);
        assert_eq!(stats.jobs_failed(), 0);
        assert_eq!(stats.jobs_queued(), 0);
        assert_eq!(stats.average_execution_time(), None);
        assert_eq!(stats.min_execution_time(), None);
        assert_eq!(stats.max_execution_time(), None);
        assert!(stats.jobs_by_priority().is_empty());
        assert_eq!(stats.completion_rate(), 0.0);
    }
    
    #[test]
    fn test_register_job_submission() {
        let stats = JobPoolStats::new();
        
        stats.register_job_submission();
        assert_eq!(stats.jobs_submitted(), 1);
        assert_eq!(stats.jobs_queued(), 1);
        
        stats.register_job_submissions(5);
        assert_eq!(stats.jobs_submitted(), 6);
        assert_eq!(stats.jobs_queued(), 6);
    }
    
    #[test]
    fn test_register_job_completed() {
        let stats = JobPoolStats::new();
        
        // Register submissions
        stats.register_job_submissions(10);
        
        // Register 7 successful completions
        for i in 0..7 {
            let duration = Duration::from_millis(100 + i * 50);
            stats.register_job_completed(true, duration, Some(JobPriority::Normal));
        }
        
        // Register 2 failures
        for i in 0..2 {
            let duration = Duration::from_millis(200 + i * 50);
            stats.register_job_completed(false, duration, Some(JobPriority::High));
        }
        
        assert_eq!(stats.jobs_succeeded(), 7);
        assert_eq!(stats.jobs_failed(), 2);
        assert_eq!(stats.jobs_queued(), 1); // 10 submitted - 9 completed
        
        // Check timing stats
        assert!(stats.average_execution_time().is_some());
        assert!(stats.min_execution_time().is_some());
        assert!(stats.max_execution_time().is_some());
        
        // Verify min and max times
        assert_eq!(stats.min_execution_time(), Some(Duration::from_millis(100)));
        assert_eq!(stats.max_execution_time(), Some(Duration::from_millis(400)));
        
        // Check priority stats
        let by_priority = stats.jobs_by_priority();
        assert_eq!(by_priority.get(&JobPriority::Normal), Some(&7));
        assert_eq!(by_priority.get(&JobPriority::High), Some(&2));
        
        // Check completion rate
        assert_eq!(stats.completion_rate(), 0.7); // 7 out of 10
    }
    
    #[test]
    fn test_reset_stats() {
        let stats = JobPoolStats::new();
        
        // Add some data
        stats.register_job_submissions(5);
        stats.register_job_completed(true, Duration::from_millis(100), Some(JobPriority::High));
        stats.register_job_completed(false, Duration::from_millis(200), Some(JobPriority::Low));
        
        // Reset stats
        stats.reset();
        
        // Verify everything is reset
        assert_eq!(stats.jobs_submitted(), 0);
        assert_eq!(stats.jobs_succeeded(), 0);
        assert_eq!(stats.jobs_failed(), 0);
        assert_eq!(stats.jobs_queued(), 0);
        assert_eq!(stats.average_execution_time(), None);
        assert_eq!(stats.min_execution_time(), None);
        assert_eq!(stats.max_execution_time(), None);
        assert!(stats.jobs_by_priority().is_empty());
    }
    
    #[test]
    fn test_thread_safety() {
        let stats = Arc::new(JobPoolStats::new());
        let mut handles = vec![];
        
        // Spawn multiple threads updating stats
        for i in 0..10 {
            let stats_clone = stats.clone();
            let handle = thread::spawn(move || {
                // Each thread registers 100 submissions
                stats_clone.register_job_submissions(100);
                
                // Each thread registers 100 completions with varying durations
                for j in 0..100 {
                    let succeeded = j % 5 != 0; // 80% success rate
                    let duration = Duration::from_micros((100 + i * 10 + j) as u64);
                    let priority = match j % 5 {
                        0 => JobPriority::Lowest,
                        1 => JobPriority::Low,
                        2 => JobPriority::Normal,
                        3 => JobPriority::High,
                        _ => JobPriority::Highest,
                    };
                    
                    stats_clone.register_job_completed(succeeded, duration, Some(priority));
                }
            });
            
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Check final stats
        assert_eq!(stats.jobs_submitted(), 1000); // 10 threads * 100 submissions
        assert_eq!(stats.jobs_succeeded(), 800);  // 80% success rate
        assert_eq!(stats.jobs_failed(), 200);     // 20% failure rate
        assert_eq!(stats.jobs_queued(), 0);       // All jobs completed
        
        // Verify priority distribution (each priority should have ~200 jobs)
        let by_priority = stats.jobs_by_priority();
        for priority in [
            JobPriority::Lowest,
            JobPriority::Low,
            JobPriority::Normal,
            JobPriority::High,
            JobPriority::Highest,
        ] {
            assert!(by_priority.contains_key(&priority));
            assert_eq!(*by_priority.get(&priority).unwrap(), 200);
        }
    }
    
    #[test]
    fn test_stats_snapshot() {
        let stats = JobPoolStats::new();
        
        // Add some data
        stats.register_job_submissions(100);
        for i in 0..80 {
            let duration = Duration::from_millis(i as u64 + 50);
            let succeeded = i % 4 != 0; // 75% success rate
            stats.register_job_completed(succeeded, duration, Some(JobPriority::Normal));
        }
        
        // Create snapshot
        let snapshot = JobPoolStatsSnapshot::from_stats(&stats);
        
        // Verify snapshot data
        assert_eq!(snapshot.jobs_submitted, 100);
        assert_eq!(snapshot.jobs_succeeded, 60);
        assert_eq!(snapshot.jobs_failed, 20);
        assert_eq!(snapshot.jobs_queued, 20);
        assert!(snapshot.average_execution_time.is_some());
        assert!(snapshot.min_execution_time.is_some());
        assert!(snapshot.max_execution_time.is_some());
        assert_eq!(snapshot.min_execution_time, Some(Duration::from_millis(50)));
        assert_eq!(snapshot.max_execution_time, Some(Duration::from_millis(129)));
        assert_eq!(snapshot.completion_rate, 0.6); // 60 / 100
        
        // Now modify the original stats
        stats.register_job_submissions(50);
        
        // Snapshot should be unchanged
        assert_eq!(snapshot.jobs_submitted, 100);
        assert_eq!(snapshot.jobs_queued, 20);
    }
}