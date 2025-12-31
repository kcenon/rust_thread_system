//! Typed job trait and wrappers for type-aware job routing.
//!
//! This module provides [`TypedJob`] for jobs that declare their type,
//! and [`TypedClosureJob`] for wrapping closures with an explicit type.

use super::JobType;
use crate::core::{Job, Result};

/// Trait for jobs that declare their type for routing decisions.
///
/// Jobs implementing this trait can be submitted to [`TypedThreadPool`]
/// and will be automatically routed to the appropriate type-specific queue.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::core::{Job, Result};
/// use rust_thread_system::typed::{JobType, TypedJob, DefaultJobType};
///
/// struct DatabaseQuery {
///     query: String,
/// }
///
/// impl Job for DatabaseQuery {
///     fn execute(&mut self) -> Result<()> {
///         // Execute database query
///         Ok(())
///     }
///
///     fn job_type(&self) -> &str {
///         "DatabaseQuery"
///     }
/// }
///
/// impl TypedJob<DefaultJobType> for DatabaseQuery {
///     fn typed_job_type(&self) -> DefaultJobType {
///         DefaultJobType::Io
///     }
/// }
/// ```
///
/// [`TypedThreadPool`]: crate::typed::TypedThreadPool
pub trait TypedJob<T: JobType>: Job {
    /// Returns the job type for routing decisions.
    ///
    /// This determines which queue the job will be placed in when submitted
    /// to a [`TypedThreadPool`].
    ///
    /// [`TypedThreadPool`]: crate::typed::TypedThreadPool
    fn typed_job_type(&self) -> T;
}

/// A wrapper that associates a closure with a specific job type.
///
/// This allows submitting closures to [`TypedThreadPool`] with explicit
/// type routing.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::typed::{TypedClosureJob, DefaultJobType};
///
/// let job = TypedClosureJob::new(DefaultJobType::Io, || {
///     // IO-bound work
///     Ok(())
/// });
/// ```
///
/// [`TypedThreadPool`]: crate::typed::TypedThreadPool
pub struct TypedClosureJob<T, F>
where
    T: JobType,
    F: FnOnce() -> Result<()> + Send,
{
    job_type: T,
    closure: Option<F>,
    name: String,
}

impl<T, F> TypedClosureJob<T, F>
where
    T: JobType,
    F: FnOnce() -> Result<()> + Send,
{
    /// Creates a new typed closure job.
    ///
    /// # Arguments
    ///
    /// * `job_type` - The type category for routing
    /// * `closure` - The closure to execute
    pub fn new(job_type: T, closure: F) -> Self {
        let name = format!("TypedClosureJob<{}>", job_type.name());
        Self {
            job_type,
            closure: Some(closure),
            name,
        }
    }

    /// Creates a new typed closure job with a custom name.
    ///
    /// # Arguments
    ///
    /// * `job_type` - The type category for routing
    /// * `closure` - The closure to execute
    /// * `name` - Custom name for debugging and statistics
    pub fn with_name<S: Into<String>>(job_type: T, closure: F, name: S) -> Self {
        Self {
            job_type,
            closure: Some(closure),
            name: name.into(),
        }
    }
}

impl<T, F> Job for TypedClosureJob<T, F>
where
    T: JobType,
    F: FnOnce() -> Result<()> + Send,
{
    fn execute(&mut self) -> Result<()> {
        if let Some(closure) = self.closure.take() {
            closure()
        } else {
            Err(crate::core::ThreadError::other(
                "TypedClosureJob already executed - cannot execute twice",
            ))
        }
    }

    fn job_type(&self) -> &str {
        &self.name
    }
}

impl<T, F> TypedJob<T> for TypedClosureJob<T, F>
where
    T: JobType,
    F: FnOnce() -> Result<()> + Send,
{
    fn typed_job_type(&self) -> T {
        self.job_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed::DefaultJobType;

    #[test]
    fn test_typed_closure_job_creation() {
        let job = TypedClosureJob::new(DefaultJobType::Io, || Ok(()));
        assert_eq!(job.typed_job_type(), DefaultJobType::Io);
        assert!(job.job_type().contains("IO"));
    }

    #[test]
    fn test_typed_closure_job_with_name() {
        let job = TypedClosureJob::with_name(DefaultJobType::Compute, || Ok(()), "CustomName");
        assert_eq!(job.job_type(), "CustomName");
        assert_eq!(job.typed_job_type(), DefaultJobType::Compute);
    }

    #[test]
    fn test_typed_closure_job_execute() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = Arc::clone(&executed);

        let mut job = TypedClosureJob::new(DefaultJobType::Compute, move || {
            executed_clone.store(true, Ordering::SeqCst);
            Ok(())
        });

        assert!(!executed.load(Ordering::SeqCst));
        job.execute().unwrap();
        assert!(executed.load(Ordering::SeqCst));
    }

    #[test]
    fn test_typed_closure_job_execute_twice_fails() {
        let mut job = TypedClosureJob::new(DefaultJobType::Io, || Ok(()));
        assert!(job.execute().is_ok());
        assert!(job.execute().is_err());
    }

    struct CustomTypedJob {
        job_type: DefaultJobType,
        value: i32,
    }

    impl Job for CustomTypedJob {
        fn execute(&mut self) -> Result<()> {
            self.value += 1;
            Ok(())
        }

        fn job_type(&self) -> &str {
            "CustomTypedJob"
        }
    }

    impl TypedJob<DefaultJobType> for CustomTypedJob {
        fn typed_job_type(&self) -> DefaultJobType {
            self.job_type
        }
    }

    #[test]
    fn test_custom_typed_job() {
        let job = CustomTypedJob {
            job_type: DefaultJobType::Critical,
            value: 0,
        };
        assert_eq!(job.typed_job_type(), DefaultJobType::Critical);
    }
}
