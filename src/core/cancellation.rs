//! Job cancellation infrastructure
//!
//! This module provides a thread-safe cancellation mechanism for jobs,
//! supporting hierarchical (parent-child) relationships, callback registration,
//! and automatic timeout cancellation.
//!
//! # Features
//!
//! - **Hierarchical cancellation**: Create child tokens that are automatically
//!   cancelled when their parent is cancelled
//! - **Timeout cancellation**: Create tokens that auto-cancel after a specified duration
//! - **Cancellation callbacks**: Register callbacks to run when a token is cancelled
//! - **Cancellation reasons**: Track why a token was cancelled
//!
//! # Example
//!
//! ```rust
//! use rust_thread_system::CancellationToken;
//! use std::time::Duration;
//!
//! // Create a parent token
//! let parent = CancellationToken::new();
//!
//! // Create child tokens
//! let child1 = parent.child();
//! let child2 = parent.child();
//!
//! // Cancel parent - all children are also cancelled
//! parent.cancel();
//!
//! assert!(parent.is_cancelled());
//! assert!(child1.is_cancelled());
//! assert!(child2.is_cancelled());
//! ```

use crate::core::Result;
use crate::ThreadError;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_CALLBACK_ID: AtomicUsize = AtomicUsize::new(1);

/// Generates a unique job ID
fn next_job_id() -> u64 {
    NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed)
}

/// Generates a unique callback ID
fn next_callback_id() -> usize {
    NEXT_CALLBACK_ID.fetch_add(1, Ordering::Relaxed)
}

/// Reason for cancellation
///
/// This enum describes why a cancellation token was cancelled, which is useful
/// for logging, debugging, and conditional cleanup logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CancellationReason {
    /// Explicitly cancelled by user via `cancel()` or `cancel_with_reason()`
    Manual,
    /// Cancelled due to timeout expiration
    Timeout(Duration),
    /// Cancelled because the parent token was cancelled
    ParentCancelled,
    /// Cancelled due to an error condition
    Error(String),
    /// Custom cancellation reason
    Custom(String),
}

impl std::fmt::Display for CancellationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CancellationReason::Manual => write!(f, "manually cancelled"),
            CancellationReason::Timeout(d) => write!(f, "timeout after {:?}", d),
            CancellationReason::ParentCancelled => write!(f, "parent was cancelled"),
            CancellationReason::Error(msg) => write!(f, "error: {}", msg),
            CancellationReason::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

/// Stored callback with its ID for later removal
struct StoredCallback {
    id: usize,
    callback: Box<dyn FnOnce() + Send + Sync>,
}

/// Internal state for a cancellation token
struct CancellationTokenInner {
    /// Cancellation state
    cancelled: AtomicBool,
    /// Child tokens (weak references to avoid cycles)
    children: RwLock<Vec<Weak<CancellationTokenInner>>>,
    /// Registered callbacks
    callbacks: RwLock<Vec<StoredCallback>>,
    /// Cancellation reason
    reason: RwLock<Option<CancellationReason>>,
}

impl std::fmt::Debug for CancellationTokenInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationTokenInner")
            .field("cancelled", &self.cancelled.load(Ordering::Relaxed))
            .field("children_count", &self.children.read().len())
            .field("callbacks_count", &self.callbacks.read().len())
            .field("reason", &*self.reason.read())
            .finish()
    }
}

/// A thread-safe cancellation token that can be shared between a job and its caller
///
/// # Features
///
/// - **Hierarchical cancellation**: Create child tokens using [`child()`](Self::child) that are
///   automatically cancelled when the parent is cancelled
/// - **Timeout cancellation**: Create tokens that auto-cancel after a duration using
///   [`with_timeout()`](Self::with_timeout)
/// - **Cancellation callbacks**: Register cleanup callbacks using [`on_cancel()`](Self::on_cancel)
/// - **Cancellation reasons**: Track why cancellation occurred via [`reason()`](Self::reason)
///
/// # Example
///
/// ```rust
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
///
/// # Hierarchical Example
///
/// ```rust
/// use rust_thread_system::CancellationToken;
///
/// let parent = CancellationToken::new();
/// let child1 = parent.child();
/// let child2 = parent.child();
///
/// // Child tokens start uncancelled
/// assert!(!child1.is_cancelled());
/// assert!(!child2.is_cancelled());
///
/// // Cancelling parent cancels all children
/// parent.cancel();
/// assert!(child1.is_cancelled());
/// assert!(child2.is_cancelled());
/// ```
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationTokenInner>,
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .field("reason", &self.reason())
            .finish()
    }
}

impl CancellationToken {
    /// Create a new cancellation token (not cancelled)
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationTokenInner {
                cancelled: AtomicBool::new(false),
                children: RwLock::new(Vec::new()),
                callbacks: RwLock::new(Vec::new()),
                reason: RwLock::new(None),
            }),
        }
    }

    /// Creates a child token linked to this parent
    ///
    /// The child token is automatically cancelled when the parent is cancelled.
    /// If the parent is already cancelled when this is called, the child is
    /// created in a cancelled state.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::CancellationToken;
    ///
    /// let parent = CancellationToken::new();
    /// let child = parent.child();
    ///
    /// // Cancelling parent cancels child
    /// parent.cancel();
    /// assert!(child.is_cancelled());
    /// ```
    pub fn child(&self) -> Self {
        let child = CancellationToken {
            inner: Arc::new(CancellationTokenInner {
                cancelled: AtomicBool::new(false),
                children: RwLock::new(Vec::new()),
                callbacks: RwLock::new(Vec::new()),
                reason: RwLock::new(None),
            }),
        };

        // Register with parent
        self.inner
            .children
            .write()
            .push(Arc::downgrade(&child.inner));

        // If parent already cancelled, cancel child immediately
        if self.is_cancelled() {
            child.cancel_with_reason(CancellationReason::ParentCancelled);
        }

        child
    }

    /// Creates a token that auto-cancels after the specified timeout
    ///
    /// A background thread is spawned to trigger the cancellation.
    /// If the token is cancelled manually before the timeout, the
    /// timeout thread will exit without effect.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::{CancellationToken, CancellationReason};
    /// use std::time::Duration;
    /// use std::thread;
    ///
    /// let token = CancellationToken::with_timeout(Duration::from_millis(50));
    /// assert!(!token.is_cancelled());
    ///
    /// thread::sleep(Duration::from_millis(100));
    /// assert!(token.is_cancelled());
    /// assert_eq!(token.reason(), Some(CancellationReason::Timeout(Duration::from_millis(50))));
    /// ```
    pub fn with_timeout(timeout: Duration) -> Self {
        let token = Self::new();
        let token_clone = token.clone();

        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            // Only cancel if not already cancelled
            if !token_clone.is_cancelled() {
                token_clone.cancel_with_reason(CancellationReason::Timeout(timeout));
            }
        });

        token
    }

    /// Creates a child token that auto-cancels after the specified timeout
    ///
    /// The token will be cancelled either when:
    /// - The parent token is cancelled, or
    /// - The timeout expires
    ///
    /// whichever happens first.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::CancellationToken;
    /// use std::time::Duration;
    ///
    /// let parent = CancellationToken::new();
    /// let child = parent.child_with_timeout(Duration::from_secs(30));
    ///
    /// // If parent cancels before timeout, child is cancelled too
    /// parent.cancel();
    /// assert!(child.is_cancelled());
    /// ```
    pub fn child_with_timeout(&self, timeout: Duration) -> Self {
        let child = self.child();
        let child_clone = child.clone();

        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            // Only cancel if not already cancelled
            if !child_clone.is_cancelled() {
                child_clone.cancel_with_reason(CancellationReason::Timeout(timeout));
            }
        });

        child
    }

    /// Cancel this token with default reason (Manual)
    ///
    /// This operation is idempotent - calling it multiple times is safe
    /// and will not cause any issues. Only the first call sets the reason.
    pub fn cancel(&self) {
        self.cancel_with_reason(CancellationReason::Manual);
    }

    /// Cancel this token with a specific reason
    ///
    /// Cancels the token and all child tokens, then executes all registered
    /// callbacks. The reason is only set on the first cancellation; subsequent
    /// calls are no-ops.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::{CancellationToken, CancellationReason};
    ///
    /// let token = CancellationToken::new();
    /// token.cancel_with_reason(CancellationReason::Error("connection lost".to_string()));
    ///
    /// assert!(token.is_cancelled());
    /// assert_eq!(
    ///     token.reason(),
    ///     Some(CancellationReason::Error("connection lost".to_string()))
    /// );
    /// ```
    pub fn cancel_with_reason(&self, reason: CancellationReason) {
        // Set cancelled flag - if already cancelled, return early
        if self.inner.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }

        // Record reason
        *self.inner.reason.write() = Some(reason);

        // Execute callbacks
        let callbacks: Vec<_> = self.inner.callbacks.write().drain(..).collect();
        for stored in callbacks {
            (stored.callback)();
        }

        // Cancel all children
        let children = self.inner.children.read();
        for child_weak in children.iter() {
            if let Some(child_inner) = child_weak.upgrade() {
                let child_token = CancellationToken { inner: child_inner };
                child_token.cancel_with_reason(CancellationReason::ParentCancelled);
            }
        }
    }

    /// Check if this token has been cancelled
    ///
    /// This is a lock-free operation suitable for frequent checking
    /// in hot loops.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Returns the cancellation reason (if cancelled)
    ///
    /// Returns `None` if the token has not been cancelled.
    pub fn reason(&self) -> Option<CancellationReason> {
        self.inner.reason.read().clone()
    }

    /// Returns error if cancelled, `Ok(())` otherwise
    ///
    /// This is a convenience method for ergonomic early returns in job
    /// implementations using the `?` operator.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::{CancellationToken, ThreadError};
    ///
    /// fn process_items(token: &CancellationToken) -> Result<(), ThreadError> {
    ///     for i in 0..100 {
    ///         token.check()?; // Returns early if cancelled
    ///         // Do work...
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            let reason_str = self
                .reason()
                .map(|r| r.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Err(ThreadError::cancelled("job", reason_str))
        } else {
            Ok(())
        }
    }

    /// Registers a callback to run when cancelled
    ///
    /// Returns a guard that unregisters the callback when dropped.
    /// If the token is already cancelled when this is called, the callback
    /// is executed immediately.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::CancellationToken;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    /// use std::sync::Arc;
    ///
    /// let token = CancellationToken::new();
    /// let called = Arc::new(AtomicBool::new(false));
    /// let called_clone = Arc::clone(&called);
    ///
    /// let _guard = token.on_cancel(move || {
    ///     called_clone.store(true, Ordering::SeqCst);
    /// });
    ///
    /// token.cancel();
    /// assert!(called.load(Ordering::SeqCst));
    /// ```
    pub fn on_cancel<F>(&self, callback: F) -> CancellationCallbackGuard
    where
        F: FnOnce() + Send + Sync + 'static,
    {
        let id = next_callback_id();

        if self.is_cancelled() {
            // Already cancelled, run immediately
            callback();
        } else {
            self.inner.callbacks.write().push(StoredCallback {
                id,
                callback: Box::new(callback),
            });
        }

        CancellationCallbackGuard {
            token: Some(self.clone()),
            callback_id: id,
        }
    }

    /// Registers callback without guard (always runs when cancelled)
    ///
    /// Unlike [`on_cancel()`](Self::on_cancel), this method does not return a guard,
    /// so the callback cannot be unregistered. The callback will always run
    /// when the token is cancelled (or immediately if already cancelled).
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::CancellationToken;
    ///
    /// let token = CancellationToken::new();
    ///
    /// token.on_cancel_always(|| {
    ///     println!("Token was cancelled, performing cleanup...");
    /// });
    ///
    /// token.cancel(); // Callback runs here
    /// ```
    pub fn on_cancel_always<F>(&self, callback: F)
    where
        F: FnOnce() + Send + Sync + 'static,
    {
        self.on_cancel(callback).detach();
    }

    /// Removes a callback by its ID
    fn remove_callback(&self, callback_id: usize) {
        self.inner.callbacks.write().retain(|c| c.id != callback_id);
    }

    /// Reset the cancellation state (for reusing tokens)
    ///
    /// This clears the cancelled flag, reason, and all callbacks.
    /// Children are not affected by reset.
    ///
    /// **Warning**: This is primarily useful for testing. In production code,
    /// it's generally better to create a new token rather than resetting
    /// an existing one.
    pub fn reset(&self) {
        self.inner.cancelled.store(false, Ordering::Release);
        *self.inner.reason.write() = None;
        self.inner.callbacks.write().clear();
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Guard that unregisters a callback when dropped
///
/// This guard is returned by [`CancellationToken::on_cancel()`] and will
/// remove the associated callback when it goes out of scope, unless the
/// callback has already been executed due to cancellation.
///
/// # Example
///
/// ```rust
/// use rust_thread_system::CancellationToken;
///
/// let token = CancellationToken::new();
///
/// {
///     let _guard = token.on_cancel(|| {
///         println!("This won't run if guard is dropped first");
///     });
///     // Guard is dropped here, callback is unregistered
/// }
///
/// token.cancel(); // Callback does NOT run
/// ```
pub struct CancellationCallbackGuard {
    token: Option<CancellationToken>,
    callback_id: usize,
}

impl Drop for CancellationCallbackGuard {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            // Only remove if token hasn't been cancelled
            // (if cancelled, callback was already executed and removed)
            if !token.is_cancelled() {
                token.remove_callback(self.callback_id);
            }
        }
    }
}

impl CancellationCallbackGuard {
    /// Detaches the guard, preventing the callback from being unregistered
    ///
    /// After calling this method, the callback will remain registered
    /// even when the guard is dropped.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_thread_system::CancellationToken;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    /// use std::sync::Arc;
    ///
    /// let token = CancellationToken::new();
    /// let called = Arc::new(AtomicBool::new(false));
    /// let called_clone = Arc::clone(&called);
    ///
    /// {
    ///     let guard = token.on_cancel(move || {
    ///         called_clone.store(true, Ordering::SeqCst);
    ///     });
    ///     guard.detach(); // Callback will remain registered
    /// }
    ///
    /// token.cancel();
    /// assert!(called.load(Ordering::SeqCst)); // Callback ran
    /// ```
    pub fn detach(mut self) {
        self.token = None;
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
/// let pool = ThreadPool::new()?;
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
    use std::sync::atomic::AtomicUsize;
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

    // ============================================
    // Hierarchical Cancellation Token Tests
    // ============================================

    #[test]
    fn test_child_token_basic() {
        let parent = CancellationToken::new();
        let child = parent.child();

        // Both start uncancelled
        assert!(!parent.is_cancelled());
        assert!(!child.is_cancelled());

        // Cancelling parent cancels child
        parent.cancel();
        assert!(parent.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[test]
    fn test_child_token_multiple_children() {
        let parent = CancellationToken::new();
        let child1 = parent.child();
        let child2 = parent.child();
        let child3 = parent.child();

        parent.cancel();

        assert!(child1.is_cancelled());
        assert!(child2.is_cancelled());
        assert!(child3.is_cancelled());
    }

    #[test]
    fn test_child_token_independent_cancellation() {
        let parent = CancellationToken::new();
        let child1 = parent.child();
        let child2 = parent.child();

        // Cancelling one child doesn't affect parent or siblings
        child1.cancel();

        assert!(!parent.is_cancelled());
        assert!(child1.is_cancelled());
        assert!(!child2.is_cancelled());
    }

    #[test]
    fn test_child_token_nested_hierarchy() {
        let grandparent = CancellationToken::new();
        let parent = grandparent.child();
        let child = parent.child();

        // Cancelling grandparent cancels entire hierarchy
        grandparent.cancel();

        assert!(grandparent.is_cancelled());
        assert!(parent.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[test]
    fn test_child_of_already_cancelled_parent() {
        let parent = CancellationToken::new();
        parent.cancel();

        // Creating child of cancelled parent should be immediately cancelled
        let child = parent.child();
        assert!(child.is_cancelled());
        assert_eq!(child.reason(), Some(CancellationReason::ParentCancelled));
    }

    #[test]
    fn test_child_reason_is_parent_cancelled() {
        let parent = CancellationToken::new();
        let child = parent.child();

        parent.cancel();

        assert_eq!(child.reason(), Some(CancellationReason::ParentCancelled));
    }

    // ============================================
    // Timeout Cancellation Tests
    // ============================================

    #[test]
    fn test_with_timeout_basic() {
        let token = CancellationToken::with_timeout(Duration::from_millis(50));
        assert!(!token.is_cancelled());

        thread::sleep(Duration::from_millis(100));
        assert!(token.is_cancelled());
        assert_eq!(
            token.reason(),
            Some(CancellationReason::Timeout(Duration::from_millis(50)))
        );
    }

    #[test]
    fn test_with_timeout_manual_cancel_before_timeout() {
        let token = CancellationToken::with_timeout(Duration::from_secs(10));

        // Cancel manually before timeout
        token.cancel();
        assert!(token.is_cancelled());
        assert_eq!(token.reason(), Some(CancellationReason::Manual));
    }

    #[test]
    fn test_child_with_timeout_basic() {
        let parent = CancellationToken::new();
        let child = parent.child_with_timeout(Duration::from_millis(50));

        assert!(!child.is_cancelled());

        thread::sleep(Duration::from_millis(100));
        assert!(child.is_cancelled());
        assert_eq!(
            child.reason(),
            Some(CancellationReason::Timeout(Duration::from_millis(50)))
        );

        // Parent should not be affected
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn test_child_with_timeout_parent_cancels_first() {
        let parent = CancellationToken::new();
        let child = parent.child_with_timeout(Duration::from_secs(10));

        // Cancel parent before timeout
        parent.cancel();

        assert!(child.is_cancelled());
        assert_eq!(child.reason(), Some(CancellationReason::ParentCancelled));
    }

    // ============================================
    // Cancellation Reason Tests
    // ============================================

    #[test]
    fn test_cancel_with_reason_manual() {
        let token = CancellationToken::new();
        token.cancel_with_reason(CancellationReason::Manual);

        assert!(token.is_cancelled());
        assert_eq!(token.reason(), Some(CancellationReason::Manual));
    }

    #[test]
    fn test_cancel_with_reason_error() {
        let token = CancellationToken::new();
        token.cancel_with_reason(CancellationReason::Error("connection lost".to_string()));

        assert!(token.is_cancelled());
        assert_eq!(
            token.reason(),
            Some(CancellationReason::Error("connection lost".to_string()))
        );
    }

    #[test]
    fn test_cancel_with_reason_custom() {
        let token = CancellationToken::new();
        token.cancel_with_reason(CancellationReason::Custom("user requested".to_string()));

        assert!(token.is_cancelled());
        assert_eq!(
            token.reason(),
            Some(CancellationReason::Custom("user requested".to_string()))
        );
    }

    #[test]
    fn test_cancel_reason_first_wins() {
        let token = CancellationToken::new();

        // First cancellation sets the reason
        token.cancel_with_reason(CancellationReason::Manual);

        // Second cancellation is a no-op
        token.cancel_with_reason(CancellationReason::Error("should be ignored".to_string()));

        assert_eq!(token.reason(), Some(CancellationReason::Manual));
    }

    #[test]
    fn test_reason_display() {
        assert_eq!(CancellationReason::Manual.to_string(), "manually cancelled");
        assert_eq!(
            CancellationReason::Timeout(Duration::from_secs(5)).to_string(),
            "timeout after 5s"
        );
        assert_eq!(
            CancellationReason::ParentCancelled.to_string(),
            "parent was cancelled"
        );
        assert_eq!(
            CancellationReason::Error("test".to_string()).to_string(),
            "error: test"
        );
        assert_eq!(
            CancellationReason::Custom("custom msg".to_string()).to_string(),
            "custom msg"
        );
    }

    // ============================================
    // Check Method Tests
    // ============================================

    #[test]
    fn test_check_not_cancelled() {
        let token = CancellationToken::new();
        assert!(token.check().is_ok());
    }

    #[test]
    fn test_check_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.check().is_err());
    }

    #[test]
    fn test_check_error_contains_reason() {
        let token = CancellationToken::new();
        token.cancel_with_reason(CancellationReason::Error("test error".to_string()));

        let err = token.check().unwrap_err();
        assert!(err.to_string().contains("test error"));
    }

    // ============================================
    // Callback Tests
    // ============================================

    #[test]
    fn test_on_cancel_callback_executed() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let _guard = token.on_cancel(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        token.cancel();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_on_cancel_multiple_callbacks() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let counter1 = Arc::clone(&counter);
        let counter2 = Arc::clone(&counter);
        let counter3 = Arc::clone(&counter);

        let _guard1 = token.on_cancel(move || {
            counter1.fetch_add(1, Ordering::SeqCst);
        });
        let _guard2 = token.on_cancel(move || {
            counter2.fetch_add(1, Ordering::SeqCst);
        });
        let _guard3 = token.on_cancel(move || {
            counter3.fetch_add(1, Ordering::SeqCst);
        });

        token.cancel();

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_on_cancel_immediate_execution_if_already_cancelled() {
        let token = CancellationToken::new();
        token.cancel();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        // Callback should execute immediately
        let _guard = token.on_cancel(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_on_cancel_guard_unregisters_callback() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        {
            let _guard = token.on_cancel(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });
            // Guard dropped here, callback should be unregistered
        }

        token.cancel();

        // Callback should NOT have been executed
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_on_cancel_guard_detach() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        {
            let guard = token.on_cancel(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });
            guard.detach(); // Prevent unregistration
        }

        token.cancel();

        // Callback SHOULD have been executed
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_on_cancel_always() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        token.on_cancel_always(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        token.cancel();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_callbacks_executed_only_once() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let _guard = token.on_cancel(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Multiple cancel calls
        token.cancel();
        token.cancel();
        token.cancel();

        // Callback should only execute once
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    // ============================================
    // Reset Tests
    // ============================================

    #[test]
    fn test_reset_clears_reason() {
        let token = CancellationToken::new();
        token.cancel_with_reason(CancellationReason::Manual);

        assert!(token.reason().is_some());

        token.reset();

        assert!(!token.is_cancelled());
        assert!(token.reason().is_none());
    }

    #[test]
    fn test_reset_clears_callbacks() {
        let token = CancellationToken::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        token.on_cancel_always(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        token.reset();
        token.cancel();

        // Callback should not have been executed (was cleared by reset)
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    // ============================================
    // Thread Safety Tests
    // ============================================

    #[test]
    fn test_concurrent_child_creation() {
        let parent = Arc::new(CancellationToken::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let parent_clone = Arc::clone(&parent);
            let handle = thread::spawn(move || {
                let _child = parent_clone.child();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Parent should still work
        parent.cancel();
        assert!(parent.is_cancelled());
    }

    #[test]
    fn test_concurrent_cancellation() {
        let token = Arc::new(CancellationToken::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        for _ in 0..10 {
            let token_clone = Arc::clone(&token);
            let counter_clone = Arc::clone(&counter);
            let handle = thread::spawn(move || {
                let _guard = token_clone.on_cancel(move || {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                });
                thread::sleep(Duration::from_millis(50));
            });
            handles.push(handle);
        }

        // Wait for all callbacks to be registered
        thread::sleep(Duration::from_millis(25));

        // Cancel from main thread
        token.cancel();

        for handle in handles {
            handle.join().unwrap();
        }

        // All callbacks should have been executed
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }
}
