//! Custom queue implementation example
//!
//! Demonstrates how to create custom queue implementations using the JobQueue trait.
//! This example shows:
//! - Using built-in queue types (ChannelQueue, BoundedQueue)
//! - Creating a logging wrapper queue
//! - Queue capability introspection
//!
//! Run with: cargo run --example custom_queue

use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A custom queue wrapper that logs all operations.
///
/// This demonstrates how to wrap an existing queue with additional behavior.
/// You can use this pattern to add:
/// - Logging and metrics
/// - Rate limiting
/// - Custom backpressure strategies
/// - Filtering or transformation
pub struct LoggingQueue<Q: JobQueue> {
    inner: Q,
    sends: AtomicUsize,
    receives: AtomicUsize,
    name: String,
}

impl<Q: JobQueue> LoggingQueue<Q> {
    /// Creates a new logging queue wrapping the given queue.
    pub fn new(inner: Q, name: impl Into<String>) -> Self {
        Self {
            inner,
            sends: AtomicUsize::new(0),
            receives: AtomicUsize::new(0),
            name: name.into(),
        }
    }

    /// Returns the number of send operations.
    pub fn send_count(&self) -> usize {
        self.sends.load(Ordering::Relaxed)
    }

    /// Returns the number of receive operations.
    pub fn receive_count(&self) -> usize {
        self.receives.load(Ordering::Relaxed)
    }
}

impl<Q: JobQueue> JobQueue for LoggingQueue<Q> {
    fn send(&self, job: BoxedJob) -> QueueResult<()> {
        let count = self.sends.fetch_add(1, Ordering::Relaxed) + 1;
        println!(
            "  [{}] send #{}: queue_len={}",
            self.name,
            count,
            self.len()
        );
        self.inner.send(job)
    }

    fn try_send(&self, job: BoxedJob) -> QueueResult<()> {
        let count = self.sends.fetch_add(1, Ordering::Relaxed) + 1;
        let result = self.inner.try_send(job);
        match &result {
            Ok(()) => println!(
                "  [{}] try_send #{}: success, queue_len={}",
                self.name,
                count,
                self.len()
            ),
            Err(_) => println!("  [{}] try_send #{}: failed (queue full)", self.name, count),
        }
        result
    }

    fn send_timeout(&self, job: BoxedJob, timeout: Duration) -> QueueResult<()> {
        let count = self.sends.fetch_add(1, Ordering::Relaxed) + 1;
        println!(
            "  [{}] send_timeout #{}: timeout={:?}",
            self.name, count, timeout
        );
        self.inner.send_timeout(job, timeout)
    }

    fn recv(&self) -> QueueResult<BoxedJob> {
        let result = self.inner.recv();
        if result.is_ok() {
            let count = self.receives.fetch_add(1, Ordering::Relaxed) + 1;
            println!(
                "  [{}] recv #{}: queue_len={}",
                self.name,
                count,
                self.len()
            );
        }
        result
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        let result = self.inner.try_recv();
        if result.is_ok() {
            let count = self.receives.fetch_add(1, Ordering::Relaxed) + 1;
            println!(
                "  [{}] try_recv #{}: queue_len={}",
                self.name,
                count,
                self.len()
            );
        }
        result
    }

    fn recv_timeout(&self, timeout: Duration) -> QueueResult<BoxedJob> {
        let result = self.inner.recv_timeout(timeout);
        if result.is_ok() {
            let count = self.receives.fetch_add(1, Ordering::Relaxed) + 1;
            println!(
                "  [{}] recv_timeout #{}: queue_len={}",
                self.name,
                count,
                self.len()
            );
        }
        result
    }

    fn close(&self) {
        println!("  [{}] closing queue", self.name);
        self.inner.close();
    }

    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn capabilities(&self) -> QueueCapabilities {
        // Return the inner queue's capabilities
        self.inner.capabilities()
    }
}

/// A simple metrics-collecting queue that tracks job statistics.
///
/// This example shows a more practical custom queue use case.
pub struct MetricsQueue {
    inner: ChannelQueue,
    total_jobs: AtomicUsize,
    peak_queue_size: AtomicUsize,
}

impl MetricsQueue {
    /// Creates a new metrics queue.
    pub fn new() -> Self {
        Self {
            inner: ChannelQueue::unbounded(),
            total_jobs: AtomicUsize::new(0),
            peak_queue_size: AtomicUsize::new(0),
        }
    }

    /// Returns the total number of jobs that have been queued.
    pub fn total_jobs(&self) -> usize {
        self.total_jobs.load(Ordering::Relaxed)
    }

    /// Returns the peak queue size observed.
    pub fn peak_queue_size(&self) -> usize {
        self.peak_queue_size.load(Ordering::Relaxed)
    }

    fn update_peak(&self) {
        let current = self.inner.len();
        let mut peak = self.peak_queue_size.load(Ordering::Relaxed);
        while current > peak {
            match self.peak_queue_size.compare_exchange_weak(
                peak,
                current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(p) => peak = p,
            }
        }
    }
}

impl Default for MetricsQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue for MetricsQueue {
    fn send(&self, job: BoxedJob) -> QueueResult<()> {
        self.total_jobs.fetch_add(1, Ordering::Relaxed);
        let result = self.inner.send(job);
        self.update_peak();
        result
    }

    fn try_send(&self, job: BoxedJob) -> QueueResult<()> {
        self.total_jobs.fetch_add(1, Ordering::Relaxed);
        let result = self.inner.try_send(job);
        self.update_peak();
        result
    }

    fn send_timeout(&self, job: BoxedJob, timeout: Duration) -> QueueResult<()> {
        self.total_jobs.fetch_add(1, Ordering::Relaxed);
        let result = self.inner.send_timeout(job, timeout);
        self.update_peak();
        result
    }

    fn recv(&self) -> QueueResult<BoxedJob> {
        self.inner.recv()
    }

    fn try_recv(&self) -> QueueResult<BoxedJob> {
        self.inner.try_recv()
    }

    fn recv_timeout(&self, timeout: Duration) -> QueueResult<BoxedJob> {
        self.inner.recv_timeout(timeout)
    }

    fn close(&self) {
        self.inner.close();
    }

    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn capabilities(&self) -> QueueCapabilities {
        self.inner.capabilities()
    }
}

fn main() -> Result<()> {
    println!("=== Rust Thread System - Custom Queue Example ===\n");

    // Part 1: Using built-in queues
    println!("1. Built-in Queue Types:");
    println!("   ChannelQueue - Unbounded FIFO queue (default)");
    println!("   BoundedQueue - Bounded FIFO with capacity limit");
    println!("   PriorityJobQueue - Priority-based (feature-gated)");

    // Part 2: Queue capability introspection
    println!("\n2. Queue Capability Introspection:");

    let channel_queue = ChannelQueue::unbounded();
    let bounded_queue = BoundedQueue::new(100);

    print_capabilities("ChannelQueue", channel_queue.capabilities());
    print_capabilities("BoundedQueue", bounded_queue.capabilities());

    // Part 3: Using a custom logging queue
    println!("\n3. Custom Logging Queue:");

    let logging_queue = Arc::new(LoggingQueue::new(ChannelQueue::unbounded(), "LogQueue"));

    let config = ThreadPoolConfig::new(2).with_queue(logging_queue.clone() as Arc<dyn JobQueue>);
    let pool = ThreadPool::with_config(config)?;
    pool.start()?;

    println!("   Submitting 5 jobs through logging queue:");
    for i in 0..5 {
        pool.execute(move || {
            thread::sleep(Duration::from_millis(50));
            println!("    -> Job {} completed", i);
            Ok(())
        })?;
    }

    // Wait for jobs to complete
    thread::sleep(Duration::from_millis(400));

    println!(
        "\n   Logging queue stats: sends={}, receives={}",
        logging_queue.send_count(),
        logging_queue.receive_count()
    );

    pool.shutdown()?;

    // Part 4: Metrics queue example
    println!("\n4. Metrics Queue Example:");

    let metrics_queue = Arc::new(MetricsQueue::new());
    let config = ThreadPoolConfig::new(2).with_queue(metrics_queue.clone() as Arc<dyn JobQueue>);
    let pool = ThreadPool::with_config(config)?;
    pool.start()?;

    println!("   Submitting 10 jobs through metrics queue:");
    for i in 0..10 {
        pool.execute(move || {
            thread::sleep(Duration::from_millis(20));
            println!("    -> Metrics job {} completed", i);
            Ok(())
        })?;
    }

    thread::sleep(Duration::from_millis(300));

    println!(
        "\n   Metrics: total_jobs={}, peak_queue_size={}",
        metrics_queue.total_jobs(),
        metrics_queue.peak_queue_size()
    );

    pool.shutdown()?;

    // Part 5: Demonstrating bounded queue with custom wrapper
    println!("\n5. Bounded Queue with Logging:");

    let logged_bounded = Arc::new(LoggingQueue::new(BoundedQueue::new(3), "BoundedLog"));

    let config = ThreadPoolConfig::new(1).with_queue(logged_bounded.clone() as Arc<dyn JobQueue>);
    let pool = ThreadPool::with_config(config)?;
    pool.start()?;

    println!("   Submitting jobs to bounded queue (capacity=3):");
    for i in 0..5 {
        match pool.try_execute(move || {
            thread::sleep(Duration::from_millis(100));
            Ok(())
        }) {
            Ok(()) => println!("   Job {} submitted successfully", i),
            Err(ThreadError::QueueFull { .. }) => {
                println!("   Job {} rejected - queue full", i)
            }
            Err(e) => println!("   Job {} error: {}", i, e),
        }
    }

    thread::sleep(Duration::from_millis(500));
    pool.shutdown()?;

    println!("\n=== Custom Queue Example Completed ===");
    println!("\nKey takeaways:");
    println!("  - Implement JobQueue trait for custom queue behavior");
    println!("  - Use wrapper pattern to add logging, metrics, or filtering");
    println!("  - Check capabilities() for runtime queue introspection");
    println!("  - Combine with ThreadPoolConfig::with_queue() for integration");

    Ok(())
}

/// Helper function to print queue capabilities.
fn print_capabilities(name: &str, caps: QueueCapabilities) {
    println!("   {}:", name);
    println!("     is_bounded: {}", caps.is_bounded);
    println!(
        "     capacity: {}",
        caps.capacity
            .map_or("unbounded".to_string(), |c| c.to_string())
    );
    println!("     is_lock_free: {}", caps.is_lock_free);
    println!("     supports_priority: {}", caps.supports_priority);
    println!("     exact_size: {}", caps.exact_size);
}
