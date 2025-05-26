//! Real-time metrics collection system for thread pools.
//!
//! This module provides comprehensive metrics collection and monitoring
//! capabilities for thread pool operations.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

#[cfg(feature = "metrics")]
use dashmap::DashMap;
use serde::{Serialize, Deserialize};

use crate::job::JobId;
use crate::priority_thread_pool::JobPriority;

/// Real-time metrics for thread pool operations
#[cfg(feature = "metrics")]
#[derive(Debug, Default)]
pub struct ThreadPoolMetrics {
    /// Total number of jobs submitted
    pub jobs_submitted: AtomicU64,
    
    /// Total number of jobs completed successfully
    pub jobs_completed: AtomicU64,
    
    /// Total number of jobs that failed
    pub jobs_failed: AtomicU64,
    
    /// Total number of jobs cancelled
    pub jobs_cancelled: AtomicU64,
    
    /// Total execution time in milliseconds
    pub total_execution_time_ms: AtomicU64,
    
    /// Number of currently active jobs
    pub active_jobs: AtomicUsize,
    
    /// Queue lengths by priority
    pub queue_lengths: DashMap<JobPriority, AtomicUsize>,
    
    /// Average execution time per priority
    pub avg_execution_time_ms: DashMap<JobPriority, AtomicU64>,
    
    /// Job execution start times (for calculating durations)
    pub job_start_times: DashMap<JobId, Instant>,
}

/// Snapshot of metrics at a point in time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Total number of jobs submitted
    pub jobs_submitted: u64,
    
    /// Total number of jobs completed successfully
    pub jobs_completed: u64,
    
    /// Total number of jobs that failed
    pub jobs_failed: u64,
    
    /// Total number of jobs cancelled
    pub jobs_cancelled: u64,
    
    /// Total execution time in milliseconds
    pub total_execution_time_ms: u64,
    
    /// Number of currently active jobs
    pub active_jobs: usize,
    
    /// Queue lengths by priority
    pub queue_lengths: std::collections::HashMap<JobPriority, usize>,
    
    /// Average execution time per priority (in milliseconds)
    pub avg_execution_time_ms: std::collections::HashMap<JobPriority, u64>,
    
    /// Success rate (0.0 to 1.0)
    pub success_rate: f64,
    
    /// Throughput (jobs per second based on total execution time)
    pub throughput: f64,
    
    /// Timestamp when snapshot was taken
    pub timestamp: String,
}

#[cfg(feature = "metrics")]
impl ThreadPoolMetrics {
    /// Create a new metrics instance
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Record a job submission
    pub fn record_job_submitted(&self, priority: JobPriority) {
        self.jobs_submitted.fetch_add(1, Ordering::Relaxed);
        
        // Update queue length for this priority
        self.queue_lengths
            .entry(priority)
            .or_insert_with(|| AtomicUsize::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a job starting execution
    pub fn record_job_started(&self, job_id: JobId, priority: JobPriority) {
        self.active_jobs.fetch_add(1, Ordering::Relaxed);
        self.job_start_times.insert(job_id, Instant::now());
        
        // Decrease queue length for this priority
        if let Some(queue_len) = self.queue_lengths.get(&priority) {
            queue_len.fetch_sub(1, Ordering::Relaxed);
        }
    }
    
    /// Record a job completion
    pub fn record_job_completed(&self, job_id: &JobId, priority: JobPriority) {
        self.jobs_completed.fetch_add(1, Ordering::Relaxed);
        self.active_jobs.fetch_sub(1, Ordering::Relaxed);
        
        // Calculate execution time
        if let Some((_, start_time)) = self.job_start_times.remove(job_id) {
            let execution_time = start_time.elapsed().as_millis() as u64;
            self.total_execution_time_ms.fetch_add(execution_time, Ordering::Relaxed);
            
            // Update average execution time for this priority
            self.avg_execution_time_ms
                .entry(priority)
                .or_insert_with(|| AtomicU64::new(0))
                .store(execution_time, Ordering::Relaxed);
        }
    }
    
    /// Record a job failure
    pub fn record_job_failed(&self, job_id: &JobId) {
        self.jobs_failed.fetch_add(1, Ordering::Relaxed);
        self.active_jobs.fetch_sub(1, Ordering::Relaxed);
        
        // Remove from start times
        self.job_start_times.remove(job_id);
    }
    
    /// Record a job cancellation
    pub fn record_job_cancelled(&self, job_id: &JobId) {
        self.jobs_cancelled.fetch_add(1, Ordering::Relaxed);
        self.active_jobs.fetch_sub(1, Ordering::Relaxed);
        
        // Remove from start times
        self.job_start_times.remove(job_id);
    }
    
    /// Get current queue length for a priority
    pub fn get_queue_length(&self, priority: JobPriority) -> usize {
        self.queue_lengths
            .get(&priority)
            .map(|v| v.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
    
    /// Get total queue length across all priorities
    pub fn get_total_queue_length(&self) -> usize {
        self.queue_lengths
            .iter()
            .map(|entry| entry.value().load(Ordering::Relaxed))
            .sum()
    }
    
    /// Take a snapshot of current metrics
    pub fn snapshot(&self) -> MetricsSnapshot {
        let jobs_submitted = self.jobs_submitted.load(Ordering::Relaxed);
        let jobs_completed = self.jobs_completed.load(Ordering::Relaxed);
        let jobs_failed = self.jobs_failed.load(Ordering::Relaxed);
        let jobs_cancelled = self.jobs_cancelled.load(Ordering::Relaxed);
        
        let total_finished = jobs_completed + jobs_failed + jobs_cancelled;
        let success_rate = if total_finished > 0 {
            jobs_completed as f64 / total_finished as f64
        } else {
            0.0
        };
        
        let total_execution_time_ms = self.total_execution_time_ms.load(Ordering::Relaxed);
        let throughput = if total_execution_time_ms > 0 {
            (jobs_completed as f64 * 1000.0) / total_execution_time_ms as f64
        } else {
            0.0
        };
        
        let queue_lengths = self.queue_lengths
            .iter()
            .map(|entry| (*entry.key(), entry.value().load(Ordering::Relaxed)))
            .collect();
            
        let avg_execution_time_ms = self.avg_execution_time_ms
            .iter()
            .map(|entry| (*entry.key(), entry.value().load(Ordering::Relaxed)))
            .collect();
        
        MetricsSnapshot {
            jobs_submitted,
            jobs_completed,
            jobs_failed,
            jobs_cancelled,
            total_execution_time_ms,
            active_jobs: self.active_jobs.load(Ordering::Relaxed),
            queue_lengths,
            avg_execution_time_ms,
            success_rate,
            throughput,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
    
    /// Reset all metrics to zero
    pub fn reset(&self) {
        self.jobs_submitted.store(0, Ordering::Relaxed);
        self.jobs_completed.store(0, Ordering::Relaxed);
        self.jobs_failed.store(0, Ordering::Relaxed);
        self.jobs_cancelled.store(0, Ordering::Relaxed);
        self.total_execution_time_ms.store(0, Ordering::Relaxed);
        self.active_jobs.store(0, Ordering::Relaxed);
        self.queue_lengths.clear();
        self.avg_execution_time_ms.clear();
        self.job_start_times.clear();
    }
}

/// Metrics collection disabled when feature is not enabled
#[cfg(not(feature = "metrics"))]
#[derive(Debug, Default)]
pub struct ThreadPoolMetrics;

#[cfg(not(feature = "metrics"))]
impl ThreadPoolMetrics {
    pub fn new() -> Self { Self }
    pub fn record_job_submitted(&self, _priority: JobPriority) {}
    pub fn record_job_started(&self, _job_id: JobId, _priority: JobPriority) {}
    pub fn record_job_completed(&self, _job_id: &JobId, _priority: JobPriority) {}
    pub fn record_job_failed(&self, _job_id: &JobId) {}
    pub fn record_job_cancelled(&self, _job_id: &JobId) {}
    pub fn get_queue_length(&self, _priority: JobPriority) -> usize { 0 }
    pub fn get_total_queue_length(&self) -> usize { 0 }
    pub fn reset(&self) {}
}

impl MetricsSnapshot {
    /// Print a formatted summary of the metrics
    pub fn print_summary(&self) {
        println!("=== Thread Pool Metrics Summary ===");
        println!("Timestamp: {}", self.timestamp);
        println!("Jobs submitted: {}", self.jobs_submitted);
        println!("Jobs completed: {}", self.jobs_completed);
        println!("Jobs failed: {}", self.jobs_failed);
        println!("Jobs cancelled: {}", self.jobs_cancelled);
        println!("Active jobs: {}", self.active_jobs);
        println!("Success rate: {:.2}%", self.success_rate * 100.0);
        println!("Throughput: {:.2} jobs/second", self.throughput);
        println!("Total execution time: {} ms", self.total_execution_time_ms);
        
        if !self.queue_lengths.is_empty() {
            println!("\nQueue lengths by priority:");
            let mut priorities: Vec<_> = self.queue_lengths.keys().collect();
            priorities.sort();
            for priority in priorities {
                if let Some(length) = self.queue_lengths.get(priority) {
                    println!("  {:?}: {}", priority, length);
                }
            }
        }
        
        if !self.avg_execution_time_ms.is_empty() {
            println!("\nAverage execution time by priority:");
            let mut priorities: Vec<_> = self.avg_execution_time_ms.keys().collect();
            priorities.sort();
            for priority in priorities {
                if let Some(avg_time) = self.avg_execution_time_ms.get(priority) {
                    println!("  {:?}: {} ms", priority, avg_time);
                }
            }
        }
        println!("=====================================");
    }
}