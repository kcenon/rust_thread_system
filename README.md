# Rust Thread System

> **Languages**: English | [한국어](./README.ko.md)

A production-ready, high-performance Rust threading framework with worker pools and job queues.

## Quality Status

- Verification: `cargo check`, `cargo test`(unit, integration, property, doc) ✅
- Clippy: ✅ 0 warnings
- Immediate actions: 없음
- Production guidance: 프로덕션 환경에 바로 투입 가능

## Features

- **Thread Pool Management**: Efficient worker pool with configurable thread count
- **Typed Thread Pool**: Route jobs to dedicated queues based on job type for QoS guarantees
- **Flexible Job Queues**: Support for both bounded and unbounded job queues
- **Pluggable Queue Implementations**: Custom queues via `JobQueue` trait
- **Backpressure Strategies**: Configurable handling for queue-full scenarios (block, timeout, reject, drop)
- **Priority Scheduling**: Optional priority-based job execution (feature-gated)
- **Hierarchical Cancellation**: Parent-child token relationships with cascading cancellation, timeout support, and callbacks
- **Worker Statistics**: Comprehensive metrics tracking per worker and pool-wide
- **High Performance**: Built on crossbeam channels and parking_lot for optimal performance
- **Thread Safety**: Lock-free where possible, with minimal synchronization overhead
- **Graceful Shutdown**: Clean shutdown with proper worker thread joining
- **Type-Safe Jobs**: Trait-based job system with compile-time safety
- **Custom Jobs**: Easy to implement custom job types with full control

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
rust_thread_system = "0.1.0"
```

Basic usage:

```rust
use rust_thread_system::prelude::*;

fn main() -> Result<()> {
    // Create and start a thread pool
    let pool = ThreadPool::with_threads(4)?;
    pool.start()?;

    // Submit jobs using closures
    for i in 0..10 {
        pool.execute(move || {
            println!("Job {} executing", i);
            Ok(())
        })?;
    }

    // Graceful shutdown
    pool.shutdown()?;
    Ok(())
}
```

## Architecture

### Core Components

- **Job Trait**: Define units of work to be executed
- **ThreadPool**: Manages worker threads and job distribution
- **TypedThreadPool**: Routes jobs to per-type queues for QoS guarantees
- **Worker**: Individual worker threads that process jobs
- **WorkerStats**: Per-worker statistics and metrics
- **JobQueue Trait**: Abstraction for pluggable queue implementations
- **JobType Trait**: Define job type categories for typed routing

### Design Principles

1. **Zero-cost abstractions**: Minimal overhead for job submission and execution
2. **Type safety**: Compile-time guarantees for job handling
3. **Graceful degradation**: Handle errors without crashing the pool
4. **Observable**: Rich statistics for monitoring and debugging

## Usage Examples

### Basic Thread Pool

```rust
use rust_thread_system::prelude::*;

let pool = ThreadPool::with_threads(4)?;
pool.start()?;

pool.execute(|| {
    println!("Hello from worker thread!");
    Ok(())
})?;

pool.shutdown()?;
```

### Custom Configuration

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(1000)
    .with_thread_name_prefix("my-worker")
    .with_poll_interval(Duration::from_millis(50));  // Faster responsiveness

let pool = ThreadPool::with_config(config)?;
pool.start()?;
```

#### Poll Interval Tuning

The `poll_interval` controls how frequently workers check for new jobs and shutdown signals:

- **High-throughput systems** (10-50ms): Faster job pickup, higher CPU usage
- **Background services** (500ms-1s): Lower CPU usage, slower shutdown
- **Default** (100ms): Balanced for most use cases

### Custom Job Types

```rust
use rust_thread_system::prelude::*;

struct DataProcessingJob {
    data: Vec<u32>,
}

impl Job for DataProcessingJob {
    fn execute(&mut self) -> Result<()> {
        let sum: u32 = self.data.iter().sum();
        println!("Sum: {}", sum);
        Ok(())
    }

    fn job_type(&self) -> &str {
        "DataProcessingJob"
    }
}

// Submit custom job
pool.submit(DataProcessingJob {
    data: vec![1, 2, 3, 4, 5],
})?;
```

### Bounded Queue

```rust
use rust_thread_system::prelude::*;

// Create pool with bounded queue to prevent memory exhaustion
let config = ThreadPoolConfig::new(4).with_max_queue_size(100);
let pool = ThreadPool::with_config(config)?;
pool.start()?;

// Jobs will be rejected if queue is full
match pool.execute(|| Ok(())) {
    Ok(()) => println!("Job accepted"),
    Err(ThreadError::ShuttingDown { .. }) => println!("Queue full or shutting down"),
    Err(e) => println!("Error: {}", e),
}
```

### Backpressure Strategies

Configure how the pool handles job submissions when the queue is full:

```rust
use rust_thread_system::prelude::*;
use rust_thread_system::queue::BackpressureStrategy;
use std::time::Duration;

// Strategy 1: Reject immediately (for real-time systems)
let config = ThreadPoolConfig::new(4)
    .with_max_queue_size(100)
    .reject_when_full();  // Returns error immediately when full
let pool = ThreadPool::with_config(config)?;

// Strategy 2: Block with timeout (for web servers)
let config = ThreadPoolConfig::new(4)
    .with_max_queue_size(100)
    .block_with_timeout(Duration::from_secs(5));  // Wait up to 5 seconds

// Strategy 3: Drop newest (for lossy streaming)
let config = ThreadPoolConfig::new(4)
    .with_max_queue_size(100)
    .with_backpressure_strategy(BackpressureStrategy::DropNewest);  // Silently drop

// Strategy 4: Custom handler
use rust_thread_system::queue::BackpressureHandler;
use std::sync::Arc;

struct LogAndReject;
impl BackpressureHandler for LogAndReject {
    fn handle_backpressure(&self, _job: BoxedJob) -> Result<Option<BoxedJob>> {
        eprintln!("Queue full, rejecting job");
        Err(ThreadError::queue_full(0, 0))
    }
}

let config = ThreadPoolConfig::new(4)
    .with_max_queue_size(100)
    .with_backpressure_strategy(BackpressureStrategy::Custom(Arc::new(LogAndReject)));
```

Available strategies:

| Strategy | Behavior | Use Case |
|----------|----------|----------|
| `Block` | Wait indefinitely (default) | General purpose |
| `BlockWithTimeout(Duration)` | Wait with timeout | Web servers, APIs |
| `RejectImmediately` | Return error immediately | Real-time systems |
| `DropNewest` | Silently drop new job | Lossy streaming, metrics |
| `DropOldest` | Drop oldest job (coming soon) | Latest-value semantics |
| `Custom(handler)` | User-defined logic | Complex retry logic |

### Non-Blocking Job Submission

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

let config = ThreadPoolConfig::new(4).with_max_queue_size(100);
let pool = ThreadPool::with_config(config)?;
pool.start()?;

// try_execute returns immediately if queue is full
match pool.try_execute(|| {
    println!("Job executed");
    Ok(())
}) {
    Ok(()) => println!("Job submitted"),
    Err(ThreadError::QueueFull { current, max }) => {
        println!("Queue is full ({}/{}), try again later", current, max);
    },
    Err(e) => println!("Error: {}", e),
}

// execute_timeout waits up to the specified duration for queue space
match pool.execute_timeout(|| {
    println!("Job executed");
    Ok(())
}, Duration::from_millis(100)) {
    Ok(()) => println!("Job submitted"),
    Err(ThreadError::SubmissionTimeout { timeout_ms }) => {
        println!("Submission timed out after {}ms", timeout_ms);
    },
    Err(e) => println!("Error: {}", e),
}

pool.shutdown()?;
```

### Custom Queue Implementation

Use the `JobQueue` trait to implement custom queue behavior:

```rust
use rust_thread_system::prelude::*;
use std::sync::Arc;

// Use the built-in priority queue (requires `priority-scheduling` feature)
#[cfg(feature = "priority-scheduling")]
{
    let queue = Arc::new(PriorityJobQueue::new());
    let config = ThreadPoolConfig::new(4).with_queue(queue);
    let pool = ThreadPool::with_config(config)?;
    pool.start()?;
}

// Or use ChannelQueue/BoundedQueue directly
let queue: Arc<dyn JobQueue> = Arc::new(ChannelQueue::unbounded());
let config = ThreadPoolConfig::new(4).with_queue(queue);
let pool = ThreadPool::with_config(config)?;
pool.start()?;
```

Available queue implementations:

| Queue Type | Description | Use Case |
|------------|-------------|----------|
| `ChannelQueue` | Unbounded FIFO queue | Default, high throughput |
| `BoundedQueue` | Bounded FIFO with capacity limit | Memory-constrained environments |
| `AdaptiveQueue` | Auto-switching mutex/lock-free | Variable load patterns |
| `PriorityJobQueue` | Priority-based ordering | Task prioritization (feature-gated) |

### Queue Factory

Use `QueueFactory` to create queues based on requirements without understanding implementation details:

```rust
use rust_thread_system::queue::{QueueFactory, QueueRequirements};
use std::time::Duration;

// Create a queue with specific requirements
let queue = QueueFactory::create(
    QueueRequirements::new()
        .bounded(1000)
).unwrap();

// Use convenience methods for common patterns
let high_throughput = QueueFactory::high_throughput();
let low_latency = QueueFactory::low_latency(500);
let adaptive = QueueFactory::auto_optimized();

// Use presets for specific use cases
let web_queue = QueueFactory::web_server(5000, Duration::from_secs(30));
let background_queue = QueueFactory::background_jobs();
let realtime_queue = QueueFactory::realtime(1000);
let pipeline_queue = QueueFactory::data_pipeline();
```

Integrate with `ThreadPoolConfig` using presets:

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

// Web server preset: bounded queue with timeout backpressure
let config = ThreadPoolConfig::new(8)
    .for_web_server(5000, Duration::from_secs(30));

// Background jobs preset: unbounded with priority support
let config = ThreadPoolConfig::new(4)
    .for_background_jobs();

// Real-time preset: bounded with immediate rejection
let config = ThreadPoolConfig::new(4)
    .for_realtime(1000);

// Data pipeline preset: adaptive queue for varying loads
let config = ThreadPoolConfig::new(8)
    .for_data_pipeline();

let pool = ThreadPool::with_config(config)?;
```

Queue selection matrix:

| Requirements | Selected Queue |
|-------------|----------------|
| Default | `ChannelQueue` (unbounded) |
| `bounded(N)` | `BoundedQueue` |
| `adaptive()` | `AdaptiveQueue` |
| `with_priority()` | `PriorityJobQueue` |

### Adaptive Queue

For workloads with variable contention patterns, use `AdaptiveQueue` which automatically
switches between mutex-based and lock-free strategies:

```rust
use rust_thread_system::queue::{AdaptiveQueue, AdaptiveQueueConfig, QueueStrategy, JobQueue};
use std::time::Duration;

// Configure adaptive behavior
let config = AdaptiveQueueConfig {
    high_contention_threshold: 0.7,  // Switch to lock-free above 70% contention
    low_contention_threshold: 0.3,   // Switch back to mutex below 30%
    measurement_window: 1000,        // Measure over 1000 operations
    switch_cooldown: Duration::from_millis(100),
    initial_strategy: QueueStrategy::Mutex,
};

let queue = AdaptiveQueue::new(config);

// Queue automatically adapts to workload
// - Low contention: uses efficient mutex-based queue
// - High contention: switches to lock-free queue
// - Load decreases: switches back to mutex

// Monitor queue behavior
let stats = queue.stats();
println!("Strategy: {:?}, Switches: {}", stats.current_strategy, stats.switch_count);
```

### Priority Scheduling

Enable the `priority-scheduling` feature to use priority-based job execution:

```toml
[dependencies]
rust_thread_system = { version = "0.1.0", features = ["priority-scheduling"] }
```

Use `enable_priority(true)` to automatically create a priority queue:

```rust
use rust_thread_system::prelude::*;

let config = ThreadPoolConfig::new(4).enable_priority(true);
let pool = ThreadPool::with_config(config)?;
pool.start()?;

// Submit jobs with different priorities
// Critical jobs are processed before High, High before Normal, Normal before Low
pool.execute_with_priority(|| {
    println!("Critical task - processed first");
    Ok(())
}, Priority::Critical)?;

pool.execute_with_priority(|| {
    println!("Background task - processed last");
    Ok(())
}, Priority::Low)?;

// Regular execute() uses Normal priority
pool.execute(|| {
    println!("Normal priority task");
    Ok(())
})?;

pool.shutdown()?;
```

Priority levels (highest to lowest):
- `Priority::Critical` - Must be executed ASAP
- `Priority::High` - Important tasks
- `Priority::Normal` - Default for most tasks
- `Priority::Low` - Background tasks

Within the same priority level, jobs are processed in FIFO order.

### Typed Thread Pool

For applications that need per-type QoS guarantees, use `TypedThreadPool`:

```rust
use rust_thread_system::prelude::*;

fn main() -> Result<()> {
    // Create a typed pool with per-type worker configuration
    let config = TypedPoolConfig::<DefaultJobType>::new()
        .workers_for(DefaultJobType::Critical, 4)   // Dedicated critical workers
        .workers_for(DefaultJobType::Compute, 8)    // CPU-bound work
        .workers_for(DefaultJobType::Io, 16)        // IO-bound work (high concurrency)
        .workers_for(DefaultJobType::Background, 2) // Low priority tasks
        .type_priority(vec![
            DefaultJobType::Critical,   // Processed first
            DefaultJobType::Io,
            DefaultJobType::Compute,
            DefaultJobType::Background, // Processed last
        ]);

    let pool = TypedThreadPool::new(config)?;
    pool.start()?;

    // Jobs are routed to dedicated queues based on type
    pool.execute_typed(DefaultJobType::Critical, || {
        println!("Critical task - has dedicated workers");
        Ok(())
    })?;

    pool.execute_typed(DefaultJobType::Io, || {
        println!("IO task - high concurrency");
        Ok(())
    })?;

    pool.execute_typed(DefaultJobType::Background, || {
        println!("Background task - processed when others are idle");
        Ok(())
    })?;

    // Get per-type statistics
    let stats = pool.type_stats(DefaultJobType::Io).unwrap();
    println!("IO jobs: submitted={}, completed={}, avg_latency={:?}",
        stats.jobs_submitted, stats.jobs_completed, stats.avg_latency);

    pool.shutdown()?;
    Ok(())
}
```

Use presets for common workload patterns:

```rust
// For IO-heavy workloads (databases, network services)
let config = TypedPoolConfig::<DefaultJobType>::io_optimized();

// For CPU-heavy workloads (data processing, computation)
let config = TypedPoolConfig::<DefaultJobType>::compute_optimized();

// For mixed workloads
let config = TypedPoolConfig::<DefaultJobType>::balanced();
```

Define custom job types for domain-specific categorization:

```rust
use rust_thread_system::typed::JobType;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GameJobType {
    Physics,
    Rendering,
    Audio,
    Network,
    AI,
}

impl JobType for GameJobType {
    fn all_variants() -> &'static [Self] {
        &[Self::Physics, Self::Rendering, Self::Audio, Self::Network, Self::AI]
    }

    fn default_type() -> Self {
        Self::Physics
    }
}

// Use with TypedThreadPool
let config = TypedPoolConfig::<GameJobType>::new()
    .workers_for(GameJobType::Physics, 2)
    .workers_for(GameJobType::Rendering, 4);
```

### Hierarchical Cancellation Tokens

Create cancellation tokens with parent-child relationships for cascading cancellation:

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

let pool = ThreadPool::with_threads(4)?;
pool.start()?;

// Create a cancellation scope with timeout
let scope = pool.cancellation_scope_with_timeout(Duration::from_secs(30));

// Submit multiple jobs sharing the same cancellation scope
for i in 0..5 {
    let child = scope.child();
    pool.execute_with_token(move || {
        println!("Job {} running", i);
        Ok(())
    }, child)?;
}

// Cancelling the scope cancels all child jobs
scope.cancel();

pool.shutdown()?;
```

Register callbacks to run when a token is cancelled:

```rust
use rust_thread_system::prelude::*;

let token = CancellationToken::new();

// Callback runs when token is cancelled
token.on_cancel_always(|| {
    println!("Cleaning up resources...");
});

// Cancel with a specific reason
token.cancel_with_reason(CancellationReason::Error("connection lost".to_string()));
assert!(token.is_cancelled());
```

Create tokens that auto-cancel after a timeout:

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;
use std::thread;

// Token auto-cancels after 5 seconds
let token = CancellationToken::with_timeout(Duration::from_secs(5));

// Use in a job
pool.submit_cancellable(|token| {
    while !token.is_cancelled() {
        // Do work...
        token.check()?; // Returns error if cancelled
    }
    Ok(())
})?;
```

### Worker Statistics

```rust
use rust_thread_system::prelude::*;

// Get per-worker statistics
let stats = pool.get_stats();
for (i, stat) in stats.iter().enumerate() {
    println!("Worker {}: {} jobs processed, {} failed",
        i,
        stat.get_jobs_processed(),
        stat.get_jobs_failed()
    );
    println!("  Average processing time: {:.2}μs",
        stat.get_average_processing_time_us()
    );
}

// Get pool-wide statistics
println!("Total jobs submitted: {}", pool.total_jobs_submitted());
println!("Total jobs processed: {}", pool.total_jobs_processed());
println!("Total jobs failed: {}", pool.total_jobs_failed());
```

## Examples

The `examples/` directory contains several complete examples:

- **basic_usage.rs**: Simple thread pool usage with closures
- **custom_jobs.rs**: Implementing custom job types
- **bounded_queue.rs**: Using bounded queues to limit memory usage
- **typed_pool.rs**: Using typed thread pool with per-type routing
- **custom_job_type.rs**: Defining custom job types for domain-specific categorization

Run an example:

```bash
cargo run --example basic_usage
cargo run --example custom_jobs
cargo run --example bounded_queue
cargo run --example typed_pool
cargo run --example custom_job_type
```

## Performance Characteristics

- **Job submission**: O(1) amortized
- **Worker scheduling**: Lock-free when using unbounded queues
- **Memory overhead**: Minimal - one channel per pool, not per job
- **Shutdown latency**: Bounded by longest-running job

### Benchmarks

Benchmarks can be run with:

```bash
cargo bench
```

Expected performance (on modern hardware):
- Job submission: ~1-2μs per job
- Job execution overhead: <1μs
- Throughput: Millions of jobs per second (depends on job complexity)

## Thread Safety

All public APIs are thread-safe:

- `ThreadPool` can be shared via `Arc` for multi-producer scenarios
- Job submission is lock-free for unbounded queues
- Worker statistics use atomic operations for minimal overhead

## Security

### Memory Safety

- **100% Safe Rust**: No `unsafe` code blocks in the entire codebase
- **Ownership System**: Rust's ownership model prevents data races and memory leaks
- **Type Safety**: Compile-time guarantees prevent common threading errors

### Panic Isolation

- **Per-Job Panic Recovery**: Worker threads survive job panics and continue processing
- **No Cascading Failures**: A panic in one job doesn't affect other jobs or workers
- **Statistics Tracking**: Panicked jobs are counted separately for monitoring

### DoS Protection

- **Bounded Queues**: Use `with_max_queue_size()` to prevent unbounded memory growth
- **Graceful Degradation**: Queue full errors allow applications to implement backpressure
- **Resource Limits**: Configurable thread count prevents resource exhaustion

### Best Practices

```rust
use rust_thread_system::prelude::*;

// ✅ DO: Use bounded queues in production
let config = ThreadPoolConfig::new(4)
    .with_max_queue_size(1000);  // Prevent unbounded growth
let pool = ThreadPool::with_config(config)?;

// ✅ DO: Handle queue full errors
match pool.execute(|| Ok(())) {
    Ok(()) => { /* Job submitted */ },
    Err(ThreadError::QueueFull { current, max }) => {
        // Implement backpressure - retry later or reject request
        eprintln!("Queue full ({}/{}), applying backpressure", current, max);
    },
    Err(e) => { /* Handle other errors */ },
}

// ❌ DON'T: Ignore errors
pool.execute(|| Ok(())).unwrap();  // May panic on queue full!
```

## Error Handling

The library uses a comprehensive error type:

```rust
pub enum ThreadError {
    AlreadyRunning { pool_name, worker_count },
    NotRunning { pool_name },
    ShuttingDown { pending_jobs },
    SpawnError { thread_id, message, source },
    JoinError { thread_id, message },
    ExecutionError { job_id, message },
    Cancelled { job_id, reason },
    JobTimeout { job_id, timeout_ms },
    QueueFull { current, max },
    QueueSendError,
    SubmissionTimeout { timeout_ms },
    InvalidConfig { parameter, message },
    WorkerPanic { thread_id, message },
    PoolExhausted { active, total },
    Other(String),
}
```

All errors implement `std::error::Error` via `thiserror`.

## Comparison with Alternatives

| Feature | rust_thread_system | rayon | threadpool |
|---------|-------------------|-------|------------|
| Custom job types | ✅ | ❌ | ❌ |
| Worker statistics | ✅ | ❌ | ❌ |
| Bounded queues | ✅ | N/A | ❌ |
| Graceful shutdown | ✅ | ✅ | ⚠️ |
| Data parallelism | ❌ | ✅ | ❌ |

## Dependencies

- **crossbeam**: High-performance concurrent channels
- **parking_lot**: Faster synchronization primitives
- **thiserror**: Ergonomic error handling
- **num_cpus**: CPU count detection

## License

This project is licensed under the BSD 3-Clause License. See LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## Author

Thread System Team

## See Also

- [C++ thread_system](https://github.com/kcenon/thread_system) - The original C++ implementation
- [rust_container_system](../rust_container_system) - Companion Rust container library
- [rust_database_system](../rust_database_system) - Companion Rust database library
- [rust_logger_system](../rust_logger_system) - Companion Rust logger library
