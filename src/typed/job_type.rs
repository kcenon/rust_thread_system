//! Job type definitions for typed thread pool routing.
//!
//! This module provides the [`JobType`] trait and [`DefaultJobType`] enum
//! for categorizing jobs and enabling type-based routing in [`TypedThreadPool`].
//!
//! # Custom Job Types
//!
//! You can define your own job types by implementing the [`JobType`] trait:
//!
//! ```rust
//! use rust_thread_system::typed::JobType;
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
//! enum GameJobType {
//!     Physics,
//!     Rendering,
//!     Audio,
//!     Network,
//!     AI,
//! }
//!
//! impl JobType for GameJobType {
//!     fn all_variants() -> &'static [Self] {
//!         &[Self::Physics, Self::Rendering, Self::Audio, Self::Network, Self::AI]
//!     }
//!
//!     fn default_type() -> Self {
//!         Self::Physics
//!     }
//! }
//! ```
//!
//! [`TypedThreadPool`]: crate::typed::TypedThreadPool

use std::fmt::Debug;
use std::hash::Hash;

/// Trait for defining job type categories.
///
/// Job types are used by [`TypedThreadPool`] to route jobs to dedicated queues,
/// enabling QoS guarantees and type-specific scheduling.
///
/// # Requirements
///
/// Implementations must be:
/// - `Copy + Clone`: Types are frequently copied for routing decisions
/// - `Eq + Hash`: Types are used as HashMap keys
/// - `Send + Sync + 'static`: Types are shared across threads
/// - `Debug`: Types can be formatted for logging
///
/// # Example
///
/// ```rust
/// use rust_thread_system::typed::JobType;
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// enum MyJobType {
///     Fast,
///     Slow,
/// }
///
/// impl JobType for MyJobType {
///     fn all_variants() -> &'static [Self] {
///         &[Self::Fast, Self::Slow]
///     }
///
///     fn default_type() -> Self {
///         Self::Fast
///     }
/// }
/// ```
///
/// [`TypedThreadPool`]: crate::typed::TypedThreadPool
pub trait JobType: Copy + Clone + Eq + Hash + Send + Sync + Debug + 'static {
    /// Returns all possible variants of this job type.
    ///
    /// This is used during pool initialization to create queues for each type.
    fn all_variants() -> &'static [Self];

    /// Returns the default/fallback job type.
    ///
    /// Used when submitting jobs without an explicit type via
    /// [`TypedThreadPool::execute`].
    ///
    /// [`TypedThreadPool::execute`]: crate::typed::TypedThreadPool::execute
    fn default_type() -> Self;

    /// Returns a human-readable name for this job type.
    ///
    /// Defaults to the `Debug` representation. Override for custom formatting.
    fn name(&self) -> String {
        format!("{:?}", self)
    }
}

/// Built-in job type enum for common workload categories.
///
/// This enum provides sensible defaults for most applications:
///
/// - **Io**: IO-bound operations (network, disk, database)
/// - **Compute**: CPU-intensive computations
/// - **Critical**: High-priority operations requiring low latency
/// - **Background**: Low-priority tasks that can be deferred
///
/// # Example
///
/// ```rust
/// use rust_thread_system::typed::{DefaultJobType, TypedPoolConfig};
///
/// let config = TypedPoolConfig::<DefaultJobType>::new()
///     .workers_for(DefaultJobType::Critical, 4)
///     .workers_for(DefaultJobType::Compute, 8)
///     .workers_for(DefaultJobType::Io, 16)
///     .workers_for(DefaultJobType::Background, 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DefaultJobType {
    /// IO-bound operations (network, disk, database).
    ///
    /// Recommended: Higher worker count to handle concurrent IO waits.
    Io,

    /// CPU-intensive computations.
    ///
    /// Recommended: Worker count matching CPU core count.
    #[default]
    Compute,

    /// High-priority operations requiring low latency.
    ///
    /// Recommended: Dedicated workers with priority scheduling.
    Critical,

    /// Low-priority background tasks.
    ///
    /// Recommended: Lower worker count, processed when other queues are empty.
    Background,
}

impl JobType for DefaultJobType {
    fn all_variants() -> &'static [Self] {
        &[Self::Io, Self::Compute, Self::Critical, Self::Background]
    }

    fn default_type() -> Self {
        Self::Compute
    }

    fn name(&self) -> String {
        match self {
            Self::Io => "IO".to_string(),
            Self::Compute => "Compute".to_string(),
            Self::Critical => "Critical".to_string(),
            Self::Background => "Background".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_job_type_variants() {
        let variants = DefaultJobType::all_variants();
        assert_eq!(variants.len(), 4);
        assert!(variants.contains(&DefaultJobType::Io));
        assert!(variants.contains(&DefaultJobType::Compute));
        assert!(variants.contains(&DefaultJobType::Critical));
        assert!(variants.contains(&DefaultJobType::Background));
    }

    #[test]
    fn test_default_job_type_default() {
        assert_eq!(DefaultJobType::default_type(), DefaultJobType::Compute);
        assert_eq!(DefaultJobType::default(), DefaultJobType::Compute);
    }

    #[test]
    fn test_default_job_type_name() {
        assert_eq!(DefaultJobType::Io.name(), "IO");
        assert_eq!(DefaultJobType::Compute.name(), "Compute");
        assert_eq!(DefaultJobType::Critical.name(), "Critical");
        assert_eq!(DefaultJobType::Background.name(), "Background");
    }

    #[test]
    fn test_job_type_is_copy() {
        let t = DefaultJobType::Io;
        let t2 = t;
        assert_eq!(t, t2);
    }

    #[test]
    fn test_job_type_hash() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(DefaultJobType::Io, 1);
        map.insert(DefaultJobType::Compute, 2);
        assert_eq!(map.get(&DefaultJobType::Io), Some(&1));
        assert_eq!(map.get(&DefaultJobType::Compute), Some(&2));
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum CustomJobType {
        TypeA,
        TypeB,
    }

    impl JobType for CustomJobType {
        fn all_variants() -> &'static [Self] {
            &[Self::TypeA, Self::TypeB]
        }

        fn default_type() -> Self {
            Self::TypeA
        }
    }

    #[test]
    fn test_custom_job_type() {
        assert_eq!(CustomJobType::all_variants().len(), 2);
        assert_eq!(CustomJobType::default_type(), CustomJobType::TypeA);
        assert_eq!(CustomJobType::TypeA.name(), "TypeA");
    }
}
