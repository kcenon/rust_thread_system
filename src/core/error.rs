//! Error types for the thread system

/// Result type for thread system operations
pub type Result<T> = std::result::Result<T, ThreadError>;

/// Errors that can occur in the thread system
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ThreadError {
    /// Thread pool is already running with details
    #[error("Thread pool '{pool_name}' is already running with {worker_count} workers")]
    AlreadyRunning {
        /// Name of the thread pool
        pool_name: String,
        /// Number of worker threads
        worker_count: usize,
    },

    /// Thread pool is not running
    #[error("Thread pool '{pool_name}' is not running")]
    NotRunning {
        /// Name of the thread pool
        pool_name: String,
    },

    /// Thread pool is shutting down with job count
    #[error("Thread pool is shutting down ({pending_jobs} jobs pending)")]
    ShuttingDown {
        /// Number of pending jobs
        pending_jobs: usize,
    },

    /// Failed to spawn a worker thread with details
    #[error("Failed to spawn worker thread #{thread_id}: {message}")]
    SpawnError {
        /// ID of the thread that failed to spawn
        thread_id: usize,
        /// Error message
        message: String,
        /// Source IO error
        #[source]
        source: Option<std::io::Error>,
    },

    /// Failed to join a worker thread with timeout
    #[error("Failed to join worker thread #{thread_id}: {message}")]
    JoinError {
        /// ID of the thread that failed to join
        thread_id: usize,
        /// Error message
        message: String,
    },

    /// Job execution failed with job details
    #[error("Job execution failed (job_id: {job_id}): {message}")]
    ExecutionError {
        /// ID of the failed job
        job_id: String,
        /// Error message
        message: String,
    },

    /// Job was cancelled with reason
    #[error("Job cancelled (job_id: {job_id}): {reason}")]
    Cancelled {
        /// ID of the cancelled job
        job_id: String,
        /// Reason for cancellation
        reason: String,
    },

    /// Job timeout with duration
    #[error("Job timeout after {timeout_ms}ms (job_id: {job_id})")]
    JobTimeout {
        /// ID of the timed out job
        job_id: String,
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// Queue is full with capacity details
    #[error("Job queue is full: {current}/{max} jobs queued")]
    QueueFull {
        /// Current queue size
        current: usize,
        /// Maximum queue size
        max: usize,
    },

    /// Queue send error
    #[error("Failed to send job to queue")]
    QueueSendError,

    /// Job submission timed out waiting for queue space
    #[error("Job submission timed out after {timeout_ms}ms")]
    SubmissionTimeout {
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// Invalid configuration with parameter
    #[error("Invalid configuration for '{parameter}': {message}")]
    InvalidConfig {
        /// Configuration parameter name
        parameter: String,
        /// Error message
        message: String,
    },

    /// Worker panic with details
    #[error("Worker thread #{thread_id} panicked: {message}")]
    WorkerPanic {
        /// ID of the panicked thread
        thread_id: usize,
        /// Panic message
        message: String,
    },

    /// Pool exhausted (all threads busy)
    #[error("Thread pool exhausted: {active}/{total} threads busy")]
    PoolExhausted {
        /// Number of active threads
        active: usize,
        /// Total number of threads
        total: usize,
    },

    /// General error
    #[error("{0}")]
    Other(String),
}

impl ThreadError {
    /// Create an already running error
    pub fn already_running(pool_name: impl Into<String>, worker_count: usize) -> Self {
        ThreadError::AlreadyRunning {
            pool_name: pool_name.into(),
            worker_count,
        }
    }

    /// Create a not running error
    pub fn not_running(pool_name: impl Into<String>) -> Self {
        ThreadError::NotRunning {
            pool_name: pool_name.into(),
        }
    }

    /// Create a shutting down error
    pub fn shutting_down(pending_jobs: usize) -> Self {
        ThreadError::ShuttingDown { pending_jobs }
    }

    /// Create a spawn error
    pub fn spawn(thread_id: usize, message: impl Into<String>) -> Self {
        ThreadError::SpawnError {
            thread_id,
            message: message.into(),
            source: None,
        }
    }

    /// Create a spawn error with source
    pub fn spawn_with_source(
        thread_id: usize,
        message: impl Into<String>,
        source: std::io::Error,
    ) -> Self {
        ThreadError::SpawnError {
            thread_id,
            message: message.into(),
            source: Some(source),
        }
    }

    /// Create a join error
    pub fn join(thread_id: usize, message: impl Into<String>) -> Self {
        ThreadError::JoinError {
            thread_id,
            message: message.into(),
        }
    }

    /// Create an execution error
    pub fn execution(job_id: impl Into<String>, message: impl Into<String>) -> Self {
        ThreadError::ExecutionError {
            job_id: job_id.into(),
            message: message.into(),
        }
    }

    /// Create a cancelled error
    pub fn cancelled(job_id: impl Into<String>, reason: impl Into<String>) -> Self {
        ThreadError::Cancelled {
            job_id: job_id.into(),
            reason: reason.into(),
        }
    }

    /// Create a job timeout error
    pub fn job_timeout(job_id: impl Into<String>, timeout_ms: u64) -> Self {
        ThreadError::JobTimeout {
            job_id: job_id.into(),
            timeout_ms,
        }
    }

    /// Create a queue full error
    pub fn queue_full(current: usize, max: usize) -> Self {
        ThreadError::QueueFull { current, max }
    }

    /// Create an invalid config error
    pub fn invalid_config(parameter: impl Into<String>, message: impl Into<String>) -> Self {
        ThreadError::InvalidConfig {
            parameter: parameter.into(),
            message: message.into(),
        }
    }

    /// Create a submission timeout error
    pub fn submission_timeout(timeout_ms: u64) -> Self {
        ThreadError::SubmissionTimeout { timeout_ms }
    }

    /// Create a worker panic error
    pub fn worker_panic(thread_id: usize, message: impl Into<String>) -> Self {
        ThreadError::WorkerPanic {
            thread_id,
            message: message.into(),
        }
    }

    /// Create a pool exhausted error
    pub fn pool_exhausted(active: usize, total: usize) -> Self {
        ThreadError::PoolExhausted { active, total }
    }

    /// Create a generic error
    pub fn other<S: Into<String>>(msg: S) -> Self {
        ThreadError::Other(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = ThreadError::already_running("main_pool", 8);
        assert!(matches!(err, ThreadError::AlreadyRunning { .. }));

        let err = ThreadError::queue_full(100, 100);
        assert!(matches!(err, ThreadError::QueueFull { .. }));

        let err = ThreadError::execution("job_123", "Panic in task");
        assert!(matches!(err, ThreadError::ExecutionError { .. }));
    }

    #[test]
    fn test_error_display() {
        let err = ThreadError::already_running("worker_pool", 4);
        assert_eq!(
            err.to_string(),
            "Thread pool 'worker_pool' is already running with 4 workers"
        );

        let err = ThreadError::job_timeout("job_456", 5000);
        assert_eq!(
            err.to_string(),
            "Job timeout after 5000ms (job_id: job_456)"
        );

        let err = ThreadError::pool_exhausted(8, 8);
        assert_eq!(err.to_string(), "Thread pool exhausted: 8/8 threads busy");
    }

    #[test]
    fn test_spawn_error_with_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = ThreadError::spawn_with_source(5, "Cannot create thread", io_err);

        assert!(matches!(err, ThreadError::SpawnError { .. }));
        assert!(err.to_string().contains("worker thread #5"));
    }
}
