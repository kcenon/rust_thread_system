//! Job cancellation infrastructure
//!
//! This module provides a thread-safe cancellation mechanism for jobs.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Generates a unique job ID
fn next_job_id() -> u64 {
    NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed)
}

/// A thread-safe cancellation token that can be shared between a job and its caller
///
/// # Example
///
/// ```
/// use rust_thread_system::CancellationToken;
/// use std::thread;
/// use std::time::Duration;
///
/// let token = CancellationToken::new();
/// let token_clone = token.clone();
///
/// // Spawn a thread that checks for cancellation
/// let handle = thread::spawn(move || {
///     for i in 0..10 {
///         if token_clone.is_cancelled() {
///             return "Cancelled";
///         }
///         thread::sleep(Duration::from_millis(100));
///     }
///     "Completed"
/// });
///
/// // Cancel after a short delay
/// thread::sleep(Duration::from_millis(250));
/// token.cancel();
///
/// assert_eq!(handle.join().unwrap(), "Cancelled");
/// ```
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new cancellation token (not cancelled)
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel this token
    ///
    /// This operation is idempotent - calling it multiple times is safe
    /// and will not cause any issues.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Check if this token has been cancelled
    ///
    /// This is a lock-free operation suitable for frequent checking
    /// in hot loops.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Reset the cancellation state (for reusing tokens)
    ///
    /// This is primarily useful for testing. In production code,
    /// it's generally better to create a new token rather than
    /// resetting an existing one.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::Release);
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// A handle to a submitted job that allows cancellation
///
/// The handle contains a unique job ID and a cancellation token.
/// The job must check the cancellation token periodically to
/// respect cancellation requests.
///
/// # Example
///
/// ```
/// use rust_thread_system::{ThreadPool, JobHandle, CancellationToken};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut pool = ThreadPool::new()?;
/// pool.start()?;
///
/// // FIXED: submit_cancellable requires FnOnce(CancellationToken) -> Result<()>
/// let handle = pool.submit_cancellable(|token: CancellationToken| {
///     // Job implementation that can check cancellation
///     for _i in 0..10 {
///         if token.is_cancelled() {
///             return Ok(()); // Early exit if cancelled
///         }
///         // Do work...
///     }
///     Ok(())
/// })?;
///
/// // Cancel the job
/// handle.cancel();
///
/// assert!(handle.is_cancelled());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct JobHandle {
    job_id: u64,
    token: CancellationToken,
}

impl JobHandle {
    /// Create a new job handle with a unique ID
    pub fn new() -> Self {
        Self {
            job_id: next_job_id(),
            token: CancellationToken::new(),
        }
    }

    /// Create a job handle with a specific ID (for testing)
    #[cfg(test)]
    #[allow(dead_code)] // Test utility function, available for future use
    pub(crate) fn with_id(job_id: u64) -> Self {
        Self {
            job_id,
            token: CancellationToken::new(),
        }
    }

    /// Get the unique job ID
    pub fn job_id(&self) -> u64 {
        self.job_id
    }

    /// Get a reference to the cancellation token
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Cancel this job
    ///
    /// Note: This only signals the cancellation. The job must check
    /// the token periodically to actually stop execution.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Check if this job has been cancelled
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

impl Default for JobHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_cancellation_token_creation() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_cancel() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());

        token.cancel();
        assert!(token.is_cancelled());

        // Idempotent - can call multiple times
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_reset() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());

        token.reset();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_job_handle_with_id() {
        let handle = JobHandle::with_id(42);
        assert_eq!(handle.job_id(), 42);
        assert!(!handle.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_clone() {
        let token = CancellationToken::new();
        let clone = token.clone();

        // Both should see the same state
        assert!(!token.is_cancelled());
        assert!(!clone.is_cancelled());

        // Cancelling one affects both
        token.cancel();
        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_thread_safety() {
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = thread::spawn(move || {
            for _ in 0..100 {
                if token_clone.is_cancelled() {
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            false
        });

        // Cancel after a short delay
        thread::sleep(Duration::from_millis(250));
        token.cancel();

        let was_cancelled = handle.join().unwrap();
        assert!(was_cancelled);
    }

    #[test]
    fn test_job_handle_creation() {
        let handle = JobHandle::new();
        assert!(handle.job_id() > 0);
        assert!(!handle.is_cancelled());
    }

    #[test]
    fn test_job_handle_cancel() {
        let handle = JobHandle::new();
        assert!(!handle.is_cancelled());

        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn test_job_handle_unique_ids() {
        let handle1 = JobHandle::new();
        let handle2 = JobHandle::new();
        let handle3 = JobHandle::new();

        assert_ne!(handle1.job_id(), handle2.job_id());
        assert_ne!(handle2.job_id(), handle3.job_id());
        assert_ne!(handle1.job_id(), handle3.job_id());
    }

    #[test]
    fn test_job_handle_token_sharing() {
        let handle = JobHandle::new();
        let token = handle.token().clone();

        // Both should see the same cancellation state
        assert!(!handle.is_cancelled());
        assert!(!token.is_cancelled());

        handle.cancel();
        assert!(handle.is_cancelled());
        assert!(token.is_cancelled());
    }
}
