//! Error types for the Rust Thread System library.
//!
//! This module provides a comprehensive error type system that covers all possible error 
//! conditions that might arise in the thread system's operations.

use std::io;
use std::fmt;
use std::result;
use std::sync::PoisonError;
use thiserror::Error;

/// Custom error type for the thread system
#[derive(Error, Debug)]
pub enum Error {
    /// Thread operations failed
    #[error("Thread error: {0}")]
    ThreadError(String),
    
    /// Job execution failed
    #[error("Job execution failed: {0}")]
    JobError(String),
    
    /// Job was cancelled during execution
    #[error("Job was cancelled")]
    JobCancelled,
    
    /// Job queue operations failed
    #[error("Job queue error: {0}")]
    QueueError(String),
    
    /// Thread pool operations failed
    #[error("Thread pool error: {0}")]
    ThreadPoolError(String),
    
    /// Synchronization lock failed (e.g., mutex, rwlock)
    #[error("Lock error: {0}")]
    LockError(String),
    
    /// Logger operations failed
    #[error("Logger error: {0}")]
    LoggerError(String),
    
    /// I/O operations failed
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),
    
    /// Channel operations failed
    #[error("Channel error: {0}")]
    ChannelError(String),
    
    /// Generic error 
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Create a new thread error with a custom message
    pub fn thread_error(msg: impl Into<String>) -> Self {
        Error::ThreadError(msg.into())
    }
    
    /// Create a new job error with a custom message
    pub fn job_error(msg: impl Into<String>) -> Self {
        Error::JobError(msg.into())
    }
    
    /// Create a new queue error with a custom message
    pub fn queue_error(msg: impl Into<String>) -> Self {
        Error::QueueError(msg.into())
    }
    
    /// Create a new thread pool error with a custom message
    pub fn thread_pool_error(msg: impl Into<String>) -> Self {
        Error::ThreadPoolError(msg.into())
    }
    
    /// Create a new lock error with a custom message
    pub fn lock_error(msg: impl Into<String>) -> Self {
        Error::LockError(msg.into())
    }
    
    /// Create a new logger error with a custom message
    pub fn logger_error(msg: impl Into<String>) -> Self {
        Error::LoggerError(msg.into())
    }
    
    /// Create a new channel error with a custom message
    pub fn channel_error(msg: impl Into<String>) -> Self {
        Error::ChannelError(msg.into())
    }
    
    /// Create a generic error with a custom message
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}

/// Convert PoisonError to our Error type for easier error handling with locks
impl<T> From<PoisonError<T>> for Error {
    fn from(error: PoisonError<T>) -> Self {
        Error::LockError(format!("Lock poisoned: {}", error))
    }
}

/// From string for convenient error creation
impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

/// From str for convenient error creation
impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
}

/// From Send + Any errors for general error conversion
impl From<Box<dyn std::error::Error + Send + Sync>> for Error {
    fn from(error: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Error::Other(error.to_string())
    }
}

/// Result type used throughout the library
pub type Result<T> = result::Result<T, Error>;

/// An extension to Option<String> for smoother error handling
pub trait OptionalStringExt {
    /// Convert an Option<String> to a Result with a default error message
    fn to_result<T: fmt::Display>(self, default_msg: T) -> Result<()>;
}

impl OptionalStringExt for Option<String> {
    fn to_result<T: fmt::Display>(self, _default_msg: T) -> Result<()> {
        match self {
            Some(err) => Err(Error::Other(err)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_creation() {
        let err = Error::thread_error("test error");
        assert!(matches!(err, Error::ThreadError(_)));
        
        let err = Error::from("string error");
        assert!(matches!(err, Error::Other(_)));
        
        let io_err = io::Error::new(io::ErrorKind::Other, "io error");
        let err = Error::from(io_err);
        assert!(matches!(err, Error::IoError(_)));
    }
    
    #[test]
    fn test_option_string_ext() {
        let none: Option<String> = None;
        assert!(none.to_result("default error").is_ok());
        
        let some = Some("error message".to_string());
        assert!(some.to_result("default error").is_err());
    }
}