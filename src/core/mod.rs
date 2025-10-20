//! Core types and traits for the thread system

pub mod cancellation;
pub mod error;
pub mod job;
#[cfg(feature = "priority-scheduling")]
pub mod priority;

pub use cancellation::{CancellationToken, JobHandle};
pub use error::{Result, ThreadError};
pub use job::{BoxedJob, ClosureJob, Job};
#[cfg(feature = "priority-scheduling")]
pub use priority::{Priority, PriorityJob, PriorityQueue};
