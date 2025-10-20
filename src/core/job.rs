//! Job trait and related types

use crate::core::error::Result;
use std::fmt;

/// A trait representing a unit of work to be executed by the thread pool
pub trait Job: Send {
    /// Execute the job
    ///
    /// # Errors
    ///
    /// Returns an error if the job execution fails
    fn execute(&mut self) -> Result<()>;

    /// Get the job's type name for debugging and statistics
    fn job_type(&self) -> &str {
        "Job"
    }

    /// Check if the job can be cancelled
    fn is_cancellable(&self) -> bool {
        false
    }
}

impl fmt::Debug for dyn Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Job({})", self.job_type())
    }
}

/// A boxed job that can be sent across threads
pub type BoxedJob = Box<dyn Job>;

/// Helper to create a job from a closure
pub struct ClosureJob<F>
where
    F: FnOnce() -> Result<()> + Send,
{
    closure: Option<F>,
    name: String,
}

impl<F> ClosureJob<F>
where
    F: FnOnce() -> Result<()> + Send,
{
    /// Create a new closure job
    pub fn new(closure: F) -> Self {
        Self {
            closure: Some(closure),
            name: "ClosureJob".to_string(),
        }
    }

    /// Create a new closure job with a custom name
    pub fn with_name<S: Into<String>>(closure: F, name: S) -> Self {
        Self {
            closure: Some(closure),
            name: name.into(),
        }
    }
}

impl<F> Job for ClosureJob<F>
where
    F: FnOnce() -> Result<()> + Send,
{
    fn execute(&mut self) -> Result<()> {
        if let Some(closure) = self.closure.take() {
            closure()
        } else {
            // Closure already executed, return error instead of silently succeeding
            Err(crate::core::ThreadError::other(
                "ClosureJob already executed - cannot execute twice",
            ))
        }
    }

    fn job_type(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closure_job() {
        let mut job = ClosureJob::new(|| {
            println!("Test job executed");
            Ok(())
        });

        assert_eq!(job.job_type(), "ClosureJob");
        assert!(job.execute().is_ok());
    }

    #[test]
    fn test_closure_job_with_name() {
        let job = ClosureJob::with_name(|| Ok(()), "TestJob");
        assert_eq!(job.job_type(), "TestJob");
    }
}
