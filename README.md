# Rust Thread System

A Rust implementation of a thread system for efficient and safe concurrent programming.

[![Crates.io](https://img.shields.io/crates/v/rust_thread_system.svg)](https://crates.io/crates/rust_thread_system)
[![Documentation](https://docs.rs/rust_thread_system/badge.svg)](https://docs.rs/rust_thread_system)
[![License](https://img.shields.io/crates/l/rust_thread_system.svg)](LICENSE)

## Purpose

This project provides a reusable threading system for Rust programmers to simplify complex multithreading tasks. It aims to enable efficient and safe concurrent programming through a well-designed API that leverages Rust's ownership and type system.

## Key Components

### 1. Thread Base Module

- `ThreadBase` trait: Foundational trait for thread operations
- Support for both synchronous and asynchronous execution models
- `Job` trait: Defines unit of work
- `JobQueue` trait: Manages work queue

### 2. Enhanced Logging System

- `Logger` singleton: Provides thread-safe logging functionality
- Multiple log levels (INFO, ERROR, WARN, DEBUG, TRACE) with filtering
- Dispatcher-based architecture for guaranteed message delivery
- Multiple backends: console, file, and callback
- Microsecond-precision timestamps
- Robust error recovery mechanisms
- No message loss between backends

### 3. Thread Pool System

- `ThreadWorker`: Worker thread implementation that processes jobs
- `ThreadPool`: Manages thread workers and distributes jobs
- `PauseableThreadPool`: Thread pool that can be paused and resumed
- `JobPoolStats`: Provides statistics about job execution
- Automatic thread pool shutdown when all jobs are completed

### 4. Priority-based Thread Pool

- `JobPriority`: Enum and traits for job priority levels
- `PriorityJob`: Job with priority information
- `PriorityJobQueue`: Queue that respects job priorities
- `PriorityThreadPool`: Thread pool with priority handling

### 5. Additional Features

- `BackoffStrategy`: Various backoff strategies for job retries
- `JobMonitor`: Monitors job execution status and timeouts
- `JobPersistenceManager`: Persists jobs for recovery after crashes
- `RetryableJob`: Jobs that can be retried on failure

## Key Features

- Safe concurrency using Rust's ownership and borrowing systems
- Type-safe job and worker implementations
- Comprehensive error handling with custom error types
- Flexible, high-performance logging system with guaranteed delivery
- Dynamic thread pool management with pause/resume capability
- Automatic thread pool shutdown after job completion
- Job prioritization and scheduling
- Robust retry mechanisms with configurable backoff strategies
- Performance optimized with crossbeam and parking_lot

## Usage Examples

### Basic Thread Pool

```rust
use rust_thread_system::{ThreadPool, job::CallbackJob};
use std::time::Duration;

fn main() -> Result<(), rust_thread_system::error::Error> {
    // Set up the logger
    setup_logger()?;
    
    // Create and start a thread pool with 4 workers
    let pool = ThreadPool::with_workers(4);
    pool.start()?;
    
    // Create and submit a job
    let job = CallbackJob::new(|| {
        // Log a message instead of println
        let _ = rust_thread_system::log_info!("Executing job in thread pool");
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }, "example_job");
    
    pool.submit(job)?;
    
    // Wait for the job to complete
    std::thread::sleep(Duration::from_millis(200));
    
    // Stop the thread pool
    pool.stop(true);
    
    // Stop the logger before exiting
    rust_thread_system::logger::Logger::instance().stop();
    Ok(())
}

// Set up the logger function (required for all examples)
fn setup_logger() -> Result<(), Box<dyn std::error::Error>> {
    use rust_thread_system::logger::{Logger, LoggerConfig};
    use std::path::PathBuf;
    use log::Level;
    
    let config = LoggerConfig {
        app_name: "thread_pool_example".to_string(),
        max_file_size: 1024 * 1024, // 1 MB
        use_backup: true,
        check_interval: Duration::from_millis(50),
        log_dir: PathBuf::from("logs"),
        file_level: Some(Level::Info),
        console_level: Some(Level::Info),
        callback_level: None,
    };
    
    let logger = Logger::instance();
    logger.configure(config);
    logger.start()?;
    logger.init_global_logger()?;
    
    Ok(())
}
```

### Priority Thread Pool

```rust
use rust_thread_system::{PriorityThreadPool, JobPriority, job::PriorityCallbackJob};
use std::time::Duration;

fn main() -> Result<(), rust_thread_system::error::Error> {
    // Set up the logger (see previous example)
    setup_logger()?;
    
    // Create and start a priority thread pool with 3 workers
    let pool = PriorityThreadPool::with_workers(3);
    pool.start()?;
    
    // Create and submit HIGH priority jobs
    for i in 0..5 {
        let job = PriorityCallbackJob::new(
            move || {
                let _ = rust_thread_system::log_info!("HIGH priority job {} is executing", i);
                std::thread::sleep(Duration::from_millis(100));
                let _ = rust_thread_system::log_info!("HIGH priority job {} completed", i);
                Ok(())
            },
            format!("high_priority_job_{}", i),
            JobPriority::High,
        );
        pool.submit(job)?;
    }
    
    // Create and submit MEDIUM priority jobs
    for i in 0..5 {
        let job = PriorityCallbackJob::new(
            move || {
                let _ = rust_thread_system::log_info!("MEDIUM priority job {} is executing", i);
                std::thread::sleep(Duration::from_millis(100));
                let _ = rust_thread_system::log_info!("MEDIUM priority job {} completed", i);
                Ok(())
            },
            format!("medium_priority_job_{}", i),
            JobPriority::Medium,
        );
        pool.submit(job)?;
    }
    
    // Wait for all jobs to complete
    std::thread::sleep(Duration::from_secs(3));
    
    // Stop the thread pool and logger
    pool.stop(true);
    rust_thread_system::logger::Logger::instance().stop();
    Ok(())
}
```

### Auto Shutdown Thread Pool

```rust
use rust_thread_system::{ThreadPool, job::CallbackJob};
use std::time::Duration;

fn main() -> Result<(), rust_thread_system::error::Error> {
    // Set up the logger (see previous example)
    setup_logger()?;
    
    // Create a thread pool with 4 workers and auto-shutdown enabled
    let pool = ThreadPool::with_auto_shutdown(4);
    pool.start()?;
    
    // Create and submit jobs
    for i in 0..10 {
        let job = CallbackJob::new(move || {
            let _ = rust_thread_system::log_info!("Job {} is executing", i);
            std::thread::sleep(Duration::from_millis(100));
            let _ = rust_thread_system::log_info!("Job {} completed", i);
            Ok(())
        }, format!("job_{}", i));
        
        pool.submit(job)?;
    }
    
    // No need to use std::thread::sleep() or manually call pool.stop()
    // The pool will automatically shut down when all jobs are complete
    
    // Wait for the pool to shut down on its own
    while pool.is_running() {
        std::thread::sleep(Duration::from_millis(50));
    }
    
    // Stop the logger before exiting
    rust_thread_system::logger::Logger::instance().stop();
    Ok(())
}
```

Check the [examples directory](examples/) for more detailed usage examples.

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
rust_thread_system = "0.1.0"
```

## Feature Flags

- `sync` (default): Synchronous-only operation
- `async`: Enable async/await support with tokio
- `tracing`: Enable integration with the tracing crate

## Building from Source

```bash
# Clone the repository
git clone https://github.com/username/rust_thread_system.git
cd rust_thread_system

# Build the library
cargo build --release

# Run the tests
cargo test

# Run the examples
cargo run --example basic_thread_pool
cargo run --example priority_thread_pool
cargo run --example logger_example
cargo run --example auto_shutdown_thread_pool
```

## Recent Improvements

- Replacement of println! with structured logging
- Simplified log message format with microsecond precision
- Redesigned logging architecture with dispatcher-based message delivery
- Comprehensive error handling and recovery
- Example file reorganization
- Automatic thread pool shutdown when all jobs are completed

## License

This project is licensed under the BSD 3-Clause License - see the [LICENSE](LICENSE) file for details.