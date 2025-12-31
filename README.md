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
- **Flexible Job Queues**: Support for both bounded and unbounded job queues
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
    let mut pool = ThreadPool::with_threads(4)?;
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
- **Worker**: Individual worker threads that process jobs
- **WorkerStats**: Per-worker statistics and metrics

### Design Principles

1. **Zero-cost abstractions**: Minimal overhead for job submission and execution
2. **Type safety**: Compile-time guarantees for job handling
3. **Graceful degradation**: Handle errors without crashing the pool
4. **Observable**: Rich statistics for monitoring and debugging

## Usage Examples

### Basic Thread Pool

```rust
use rust_thread_system::prelude::*;

let mut pool = ThreadPool::with_threads(4)?;
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

let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(1000)
    .with_thread_name_prefix("my-worker");

let mut pool = ThreadPool::with_config(config)?;
pool.start()?;
```

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
let mut pool = ThreadPool::with_config(config)?;
pool.start()?;

// Jobs will be rejected if queue is full
match pool.execute(|| Ok(())) {
    Ok(()) => println!("Job accepted"),
    Err(ThreadError::ShuttingDown { .. }) => println!("Queue full or shutting down"),
    Err(e) => println!("Error: {}", e),
}
```

### Non-Blocking Job Submission

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

let config = ThreadPoolConfig::new(4).with_max_queue_size(100);
let mut pool = ThreadPool::with_config(config)?;
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

Run an example:

```bash
cargo run --example basic_usage
cargo run --example custom_jobs
cargo run --example bounded_queue
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
let mut pool = ThreadPool::with_config(config)?;

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
