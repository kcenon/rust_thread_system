//! Job monitoring and health checking system.
//!
//! This module provides a monitoring system for jobs, allowing for tracking 
//! the health and status of running jobs, detecting hung jobs, and providing
//! performance metrics.

use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};
use crate::job::Job;

/// Status of a job in the monitor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// Job is waiting in queue
    Queued,
    /// Job is currently running
    Running,
    /// Job completed successfully
    Completed,
    /// Job failed
    Failed,
    /// Job timed out
    TimedOut,
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobStatus::Queued => write!(f, "Queued"),
            JobStatus::Running => write!(f, "Running"),
            JobStatus::Completed => write!(f, "Completed"),
            JobStatus::Failed => write!(f, "Failed"),
            JobStatus::TimedOut => write!(f, "Timed Out"),
        }
    }
}

/// Configuration for the job monitor
#[derive(Debug, Clone)]
pub struct JobMonitorConfig {
    /// How often to check for timed out jobs (in milliseconds)
    pub check_interval: Duration,
    /// Default timeout for jobs (in milliseconds)
    pub default_timeout: Duration,
    /// Maximum number of job records to keep
    pub max_history_size: usize,
}

impl Default for JobMonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(5),
            default_timeout: Duration::from_secs(60),
            max_history_size: 1000,
        }
    }
}

/// Information about a monitored job
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// Job ID
    pub id: usize,
    /// Job description
    pub description: String,
    /// Current status of the job
    pub status: JobStatus,
    /// When the job was created
    pub creation_time: Instant,
    /// When the job started running (if it has started)
    pub start_time: Option<Instant>,
    /// When the job completed (if it has completed)
    pub completion_time: Option<Instant>,
    /// Error message if the job failed
    pub error: Option<String>,
    /// Timeout for this job (if different from default)
    pub timeout: Duration,
}

impl JobInfo {
    /// Create a new job info record
    fn new(id: usize, job: &dyn Job, timeout: Duration) -> Self {
        Self {
            id,
            description: job.description(),
            status: JobStatus::Queued,
            creation_time: job.creation_time(),
            start_time: None,
            completion_time: None,
            error: None,
            timeout,
        }
    }
    
    /// Get the duration the job has been running (if it's running)
    pub fn running_time(&self) -> Option<Duration> {
        match (self.start_time, self.completion_time) {
            (Some(start), Some(end)) => Some(end.duration_since(start)),
            (Some(start), None) => Some(Instant::now().duration_since(start)),
            _ => None,
        }
    }
    
    /// Get the duration the job waited in the queue (if it started)
    pub fn wait_time(&self) -> Option<Duration> {
        self.start_time.map(|start| start.duration_since(self.creation_time))
    }
    
    /// Get the total time from creation to completion (if completed)
    pub fn total_time(&self) -> Option<Duration> {
        self.completion_time.map(|end| end.duration_since(self.creation_time))
    }
    
    /// Check if the job has timed out
    pub fn is_timed_out(&self) -> bool {
        if self.status != JobStatus::Running {
            return false;
        }
        
        if let Some(start) = self.start_time {
            let running_time = Instant::now().duration_since(start);
            return running_time > self.timeout;
        }
        
        false
    }
}

/// A monitor for tracking job execution
pub struct JobMonitor {
    /// Next job ID to assign
    next_id: AtomicUsize,
    /// Current jobs being monitored
    active_jobs: RwLock<HashMap<usize, JobInfo>>,
    /// History of completed jobs
    job_history: RwLock<Vec<JobInfo>>,
    /// Configuration for the monitor
    config: JobMonitorConfig,
}

impl Default for JobMonitor {
    fn default() -> Self {
        Self::new(JobMonitorConfig::default())
    }
}

impl JobMonitor {
    /// Create a new job monitor with the given configuration
    pub fn new(config: JobMonitorConfig) -> Self {
        Self {
            next_id: AtomicUsize::new(1),
            active_jobs: RwLock::new(HashMap::new()),
            job_history: RwLock::new(Vec::new()),
            config,
        }
    }
    
    /// Create a new job monitor with default configuration (using new_default for clarity)
    pub fn new_default() -> Self {
        Self::new(JobMonitorConfig::default())
    }
    
    /// Register a job with the monitor
    pub fn register_job(&self, job: &dyn Job) -> usize {
        self.register_job_with_timeout(job, self.config.default_timeout)
    }
    
    /// Register a job with a custom timeout
    pub fn register_job_with_timeout(&self, job: &dyn Job, timeout: Duration) -> usize {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let job_info = JobInfo::new(id, job, timeout);
        
        if let Ok(mut active_jobs) = self.active_jobs.write() {
            active_jobs.insert(id, job_info);
        }
        
        id
    }
    
    /// Mark a job as started
    pub fn mark_started(&self, job_id: usize) -> Result<()> {
        if let Ok(mut active_jobs) = self.active_jobs.write() {
            if let Some(job_info) = active_jobs.get_mut(&job_id) {
                job_info.status = JobStatus::Running;
                job_info.start_time = Some(Instant::now());
                return Ok(());
            }
        }
        
        Err(Error::job_error(format!("Job with ID {} not found", job_id)))
    }
    
    /// Mark a job as completed successfully
    pub fn mark_completed(&self, job_id: usize) -> Result<()> {
        if let Ok(mut active_jobs) = self.active_jobs.write() {
            if let Some(mut job_info) = active_jobs.remove(&job_id) {
                job_info.status = JobStatus::Completed;
                job_info.completion_time = Some(Instant::now());
                
                self.add_to_history(job_info);
                return Ok(());
            }
        }
        
        Err(Error::job_error(format!("Job with ID {} not found", job_id)))
    }
    
    /// Mark a job as failed with an error message
    pub fn mark_failed(&self, job_id: usize, error: impl Into<String>) -> Result<()> {
        if let Ok(mut active_jobs) = self.active_jobs.write() {
            if let Some(mut job_info) = active_jobs.remove(&job_id) {
                job_info.status = JobStatus::Failed;
                job_info.completion_time = Some(Instant::now());
                job_info.error = Some(error.into());
                
                self.add_to_history(job_info);
                return Ok(());
            }
        }
        
        Err(Error::job_error(format!("Job with ID {} not found", job_id)))
    }
    
    /// Add a job to the history, keeping history size within limits
    fn add_to_history(&self, job_info: JobInfo) {
        if let Ok(mut history) = self.job_history.write() {
            // Add the job to history
            history.push(job_info);
            
            // Trim history if needed
            if history.len() > self.config.max_history_size {
                let excess = history.len() - self.config.max_history_size;
                history.drain(0..excess);
            }
        }
    }
    
    /// Check for timed out jobs and mark them as such
    pub fn check_timeouts(&self) -> Vec<usize> {
        let mut timed_out_ids = Vec::new();
        
        if let Ok(mut active_jobs) = self.active_jobs.write() {
            let mut to_remove = Vec::new();
            
            for (id, job_info) in active_jobs.iter_mut() {
                if job_info.is_timed_out() {
                    job_info.status = JobStatus::TimedOut;
                    job_info.completion_time = Some(Instant::now());
                    job_info.error = Some("Job timed out".to_string());
                    
                    to_remove.push(*id);
                    timed_out_ids.push(*id);
                }
            }
            
            // Remove timed out jobs and add to history
            for id in to_remove {
                if let Some(job_info) = active_jobs.remove(&id) {
                    self.add_to_history(job_info);
                }
            }
        }
        
        timed_out_ids
    }
    
    /// Get information about an active job
    pub fn get_job_info(&self, job_id: usize) -> Option<JobInfo> {
        if let Ok(active_jobs) = self.active_jobs.read() {
            return active_jobs.get(&job_id).cloned();
        }
        
        None
    }
    
    /// Get all active jobs
    pub fn get_active_jobs(&self) -> Vec<JobInfo> {
        if let Ok(active_jobs) = self.active_jobs.read() {
            return active_jobs.values().cloned().collect();
        }
        
        Vec::new()
    }
    
    /// Get all jobs in the history
    pub fn get_job_history(&self) -> Vec<JobInfo> {
        if let Ok(history) = self.job_history.read() {
            return history.clone();
        }
        
        Vec::new()
    }
    
    /// Get statistics about the jobs
    pub fn get_stats(&self) -> JobStats {
        let active_jobs = self.get_active_jobs();
        let history = self.get_job_history();
        
        let mut stats = JobStats::default();
        
        // Count active jobs by status
        for job in &active_jobs {
            match job.status {
                JobStatus::Queued => stats.queued += 1,
                JobStatus::Running => stats.running += 1,
                _ => {}
            }
        }
        
        // Count completed jobs by status
        for job in &history {
            match job.status {
                JobStatus::Completed => stats.completed += 1,
                JobStatus::Failed => stats.failed += 1,
                JobStatus::TimedOut => stats.timed_out += 1,
                _ => {}
            }
        }
        
        // Calculate average times
        let (wait_time_sum, wait_count) = history.iter()
            .filter_map(|j| j.wait_time())
            .fold((Duration::from_secs(0), 0), |(sum, count), time| 
                (sum + time, count + 1)
            );
        
        let (run_time_sum, run_count) = history.iter()
            .filter_map(|j| j.running_time())
            .fold((Duration::from_secs(0), 0), |(sum, count), time| 
                (sum + time, count + 1)
            );
        
        if wait_count > 0 {
            stats.avg_wait_time = Some(wait_time_sum / wait_count as u32);
        }
        
        if run_count > 0 {
            stats.avg_run_time = Some(run_time_sum / run_count as u32);
        }
        
        stats
    }
    
    /// Clear job history
    pub fn clear_history(&self) {
        if let Ok(mut history) = self.job_history.write() {
            history.clear();
        }
    }
}

impl fmt::Debug for JobMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobMonitor")
            .field("active_jobs", &self.active_jobs.read().map(|jobs| jobs.len()).unwrap_or(0))
            .field("history_size", &self.job_history.read().map(|hist| hist.len()).unwrap_or(0))
            .field("config", &self.config)
            .finish()
    }
}

/// Statistics about jobs
#[derive(Debug, Clone, Default)]
pub struct JobStats {
    /// Number of jobs queued
    pub queued: usize,
    /// Number of jobs running
    pub running: usize,
    /// Number of jobs completed successfully
    pub completed: usize,
    /// Number of jobs failed
    pub failed: usize,
    /// Number of jobs timed out
    pub timed_out: usize,
    /// Average wait time in queue
    pub avg_wait_time: Option<Duration>,
    /// Average run time
    pub avg_run_time: Option<Duration>,
}

impl JobStats {
    /// Get the total number of jobs (active + history)
    pub fn total(&self) -> usize {
        self.queued + self.running + self.completed + self.failed + self.timed_out
    }
    
    /// Get the success rate as a percentage (0-100)
    pub fn success_rate(&self) -> f64 {
        let completed = self.completed + self.failed + self.timed_out;
        if completed == 0 {
            return 0.0;
        }
        
        (self.completed as f64 / completed as f64) * 100.0
    }
}

impl fmt::Display for JobStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Job Statistics:")?;
        writeln!(f, "  Queued:    {}", self.queued)?;
        writeln!(f, "  Running:   {}", self.running)?;
        writeln!(f, "  Completed: {}", self.completed)?;
        writeln!(f, "  Failed:    {}", self.failed)?;
        writeln!(f, "  Timed Out: {}", self.timed_out)?;
        writeln!(f, "  Total:     {}", self.total())?;
        writeln!(f, "  Success:   {:.1}%", self.success_rate())?;
        
        if let Some(avg_wait) = self.avg_wait_time {
            writeln!(f, "  Avg Wait:  {:?}", avg_wait)?;
        }
        
        if let Some(avg_run) = self.avg_run_time {
            writeln!(f, "  Avg Run:   {:?}", avg_run)?;
        }
        
        Ok(())
    }
}

/// A wrapper for jobs that are monitored
pub struct MonitoredJob<T: Job> {
    /// The actual job
    job: T,
    /// Reference to the job monitor
    monitor: Arc<JobMonitor>,
    /// ID of this job in the monitor
    job_id: usize,
}

impl<T: Job> MonitoredJob<T> {
    /// Create a new monitored job
    pub fn new(job: T, monitor: Arc<JobMonitor>) -> Self {
        let job_id = monitor.register_job(&job);
        Self { job, monitor, job_id }
    }
    
    /// Create a new monitored job with a custom timeout
    pub fn with_timeout(job: T, monitor: Arc<JobMonitor>, timeout: Duration) -> Self {
        let job_id = monitor.register_job_with_timeout(&job, timeout);
        Self { job, monitor, job_id }
    }
    
    /// Get the job ID in the monitor
    pub fn job_id(&self) -> usize {
        self.job_id
    }
}

impl<T: Job + 'static> Job for MonitoredJob<T> {
    fn execute(&self) -> Result<()> {
        // Mark job as started
        let _ = self.monitor.mark_started(self.job_id);
        
        // Execute the actual job
        match self.job.execute() {
            Ok(()) => {
                // Mark job as completed
                let _ = self.monitor.mark_completed(self.job_id);
                Ok(())
            }
            Err(e) => {
                // Mark job as failed with error message
                let _ = self.monitor.mark_failed(self.job_id, e.to_string());
                Err(e)
            }
        }
    }
    
    fn description(&self) -> String {
        self.job.description()
    }
    
    fn creation_time(&self) -> Instant {
        self.job.creation_time()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<T: Job> fmt::Debug for MonitoredJob<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MonitoredJob")
            .field("job", &self.job.description())
            .field("job_id", &self.job_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::CallbackJob;
    use std::thread;
    
    #[test]
    fn test_job_monitor_basic() {
        let monitor = JobMonitor::default();
        let job = CallbackJob::new(|| Ok(()), "test job");
        
        let job_id = monitor.register_job(&job);
        
        // Check initial state
        let job_info = monitor.get_job_info(job_id).unwrap();
        assert_eq!(job_info.status, JobStatus::Queued);
        assert_eq!(job_info.description, "test job");
        
        // Mark as started
        monitor.mark_started(job_id).unwrap();
        let job_info = monitor.get_job_info(job_id).unwrap();
        assert_eq!(job_info.status, JobStatus::Running);
        assert!(job_info.start_time.is_some());
        
        // Mark as completed
        monitor.mark_completed(job_id).unwrap();
        assert!(monitor.get_job_info(job_id).is_none());
        
        let history = monitor.get_job_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobStatus::Completed);
    }
    
    #[test]
    fn test_job_monitor_timeout() {
        let config = JobMonitorConfig {
            default_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        let monitor = JobMonitor::new(config);
        let job = CallbackJob::new(|| Ok(()), "test job");
        
        let job_id = monitor.register_job(&job);
        monitor.mark_started(job_id).unwrap();
        
        // Wait for the job to time out
        thread::sleep(Duration::from_millis(150));
        
        let timed_out = monitor.check_timeouts();
        assert_eq!(timed_out.len(), 1);
        assert_eq!(timed_out[0], job_id);
        
        assert!(monitor.get_job_info(job_id).is_none());
        
        let history = monitor.get_job_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobStatus::TimedOut);
    }
    
    #[test]
    fn test_monitored_job() {
        let monitor = Arc::new(JobMonitor::default());
        
        // Create a successful job
        let job = CallbackJob::new(|| Ok(()), "success job");
        let monitored_job = MonitoredJob::new(job, monitor.clone());
        
        // Execute the job
        assert!(monitored_job.execute().is_ok());
        
        // Check the history
        let history = monitor.get_job_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, JobStatus::Completed);
        
        // Create a failing job
        let job = CallbackJob::new(|| Err(Error::job_error("test error")), "failing job");
        let monitored_job = MonitoredJob::new(job, monitor.clone());
        
        // Execute the job
        assert!(monitored_job.execute().is_err());
        
        // Check the history
        let history = monitor.get_job_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].status, JobStatus::Failed);
        assert!(history[1].error.as_ref().unwrap().contains("test error"));
    }
}