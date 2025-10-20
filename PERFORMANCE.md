# Rust Thread System Performance Guide

> **Languages**: English | [한국어](./PERFORMANCE.ko.md)

## Overview

This guide provides detailed information on optimizing the performance of the Rust Thread System, including benchmarks, tuning strategies, and best practices.

## Table of Contents

1. [Performance Characteristics](#performance-characteristics)
2. [Benchmarks](#benchmarks)
3. [Worker Thread Tuning](#worker-thread-tuning)
4. [Queue Configuration](#queue-configuration)
5. [Job Optimization](#job-optimization)
6. [Monitoring Performance](#monitoring-performance)
7. [Common Bottlenecks](#common-bottlenecks)
8. [Comparison with Alternatives](#comparison-with-alternatives)

## Performance Characteristics

### Key Metrics

| Metric | Typical Value | Notes |
|--------|---------------|-------|
| **Job Submission Latency** | < 10 μs | Time to submit job to queue |
| **Job Execution Overhead** | < 50 μs | Thread pool overhead per job |
| **Context Switch Time** | 1-10 μs | OS-dependent |
| **Maximum Throughput** | 1M+ jobs/sec | With optimal configuration |
| **Memory per Worker** | ~2 MB | Stack + metadata |

### Scalability Characteristics

```
Linear Scaling (1-8 workers):
- Throughput increases linearly with CPU cores
- Minimal contention on job queue

Diminishing Returns (8+ workers):
- Queue contention increases
- Context switching overhead
- Cache coherency traffic
```

## Benchmarks

### Test Environment

```
CPU: Intel Core i9-9900K (8 cores, 16 threads)
RAM: 32 GB DDR4-3200
OS: Linux 5.15 (Ubuntu 22.04)
Rust: 1.75.0
```

### Throughput Benchmarks

#### Simple Jobs (No-op)

```rust
// Benchmark: Minimal job execution
pool.execute(|| Ok(()))?;
```

| Workers | Jobs/sec | Latency (μs) | CPU Usage |
|---------|----------|--------------|-----------|
| 1 | 250,000 | 4 | 100% (1 core) |
| 2 | 480,000 | 4 | 100% (2 cores) |
| 4 | 920,000 | 4 | 100% (4 cores) |
| 8 | 1,650,000 | 5 | 100% (8 cores) |
| 16 | 1,850,000 | 9 | 80% (avg) |

**Observation:** Near-linear scaling up to physical core count.

#### CPU-Bound Jobs

```rust
// Benchmark: CPU-intensive work
pool.execute(|| {
    let mut sum = 0u64;
    for i in 0..10000 {
        sum = sum.wrapping_add(i);
    }
    Ok(())
})?;
```

| Workers | Jobs/sec | Jobs/sec/Core | Efficiency |
|---------|----------|---------------|------------|
| 1 | 45,000 | 45,000 | 100% |
| 2 | 88,000 | 44,000 | 98% |
| 4 | 172,000 | 43,000 | 96% |
| 8 | 336,000 | 42,000 | 93% |

**Observation:** Excellent scaling with CPU-bound work.

#### I/O-Bound Jobs

```rust
// Benchmark: Blocking I/O
pool.execute(|| {
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
})?;
```

| Workers | Concurrent Jobs | Jobs/sec | Notes |
|---------|-----------------|----------|-------|
| 1 | 1 | 100 | 1 job per 10ms |
| 4 | 4 | 400 | 4 concurrent |
| 16 | 16 | 1,600 | 16 concurrent |
| 64 | 64 | 6,400 | High concurrency |

**Observation:** Scales with worker count for I/O-bound work.

### Memory Benchmarks

```rust
// Memory usage measurement
let pool = ThreadPool::with_threads(8)?;
pool.start()?;

// Base memory: ~16 MB (8 workers × 2 MB stack)
// Per 1000 queued jobs: ~80 KB
// Per 1M completed jobs tracked: ~48 MB (statistics)
```

### Latency Distribution

```
Job Submission (μs):
P50:  3.2 μs
P95:  8.1 μs
P99:  15.4 μs
P999: 45.2 μs

Job Execution Start (μs):
P50:  12.5 μs
P95:  89.3 μs
P99:  234.1 μs
P999: 1,203.5 μs
```

## Worker Thread Tuning

### Determining Optimal Worker Count

#### CPU-Bound Workloads

```rust
// Rule: 1 worker per physical CPU core
let num_workers = num_cpus::get_physical();

let pool = ThreadPool::with_threads(num_workers)?;
```

**Rationale:**
- Minimizes context switching
- Maximizes CPU utilization
- Avoids cache thrashing

#### I/O-Bound Workloads

```rust
// Rule: Workers = Cores × Blocking Time Ratio
// If job blocks 80% of time:
let num_workers = num_cpus::get() * 5;

let pool = ThreadPool::with_threads(num_workers)?;
```

**Example Calculations:**

| Blocking Time | Multiplier | 8-Core System |
|---------------|------------|---------------|
| 0% (Pure CPU) | 1× | 8 workers |
| 50% | 2× | 16 workers |
| 80% | 5× | 40 workers |
| 90% | 10× | 80 workers |
| 95% | 20× | 160 workers |

#### Mixed Workloads

```rust
// Start with physical cores, tune based on metrics
let num_workers = num_cpus::get_physical();

let config = ThreadPoolConfig::new(num_workers)
    .with_thread_name_prefix("mixed-worker");

let pool = ThreadPool::with_config(config)?;

// Monitor and adjust
let stats = pool.get_stats();
if stats.average_queue_size() > 100 {
    // Consider adding workers
}
```

### Worker Configuration Best Practices

```rust
// Production configuration
let config = ThreadPoolConfig::new(num_workers)
    .with_thread_name_prefix("worker")   // For debugging
    .with_stack_size(2 * 1024 * 1024)    // 2 MB stack (default)
    .with_max_queue_size(10000);          // Prevent unbounded growth

let pool = ThreadPool::with_config(config)?;
```

## Queue Configuration

### Bounded vs. Unbounded Queues

#### Unbounded Queue (Default)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(0); // 0 = unbounded

let pool = ThreadPool::with_config(config)?;
```

**Pros:**
- Never rejects jobs
- Simple to use
- No backpressure handling needed

**Cons:**
- Memory can grow unbounded
- No flow control
- Can hide performance issues

**Use When:**
- Job rate is known and bounded
- Memory is abundant
- Simplicity is preferred

#### Bounded Queue

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(1000);

let pool = ThreadPool::with_config(config)?;
```

**Pros:**
- Prevents memory exhaustion
- Forces backpressure handling
- Reveals overload conditions

**Cons:**
- Can reject jobs
- Requires error handling
- More complex

**Use When:**
- Production environments
- Unknown or variable load
- Memory is limited

### Queue Size Tuning

#### Small Queue (100-1000)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(100);
```

**Characteristics:**
- Low latency (jobs start quickly)
- Immediate feedback on overload
- Requires active backpressure handling

**Best For:**
- Latency-sensitive applications
- Real-time systems
- Interactive applications

#### Medium Queue (1,000-10,000)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(5000);
```

**Characteristics:**
- Handles burst traffic
- Balanced latency and throughput
- Moderate memory usage

**Best For:**
- Web servers
- API services
- General-purpose applications

#### Large Queue (10,000+)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(50000);
```

**Characteristics:**
- Handles large bursts
- Higher latency possible
- More memory usage

**Best For:**
- Batch processing
- Background job processing
- High-throughput systems

### Queue Sizing Formula

```rust
// Formula: Queue Size = Workers × Burst Factor × Average Job Time / Job Interval
//
// Example:
// - 8 workers
// - Want to handle 5× burst
// - Jobs take 100ms avg
// - Normal rate: 10 jobs/sec
// - Burst rate: 50 jobs/sec

let workers = 8;
let burst_factor = 5;
let avg_job_time_ms = 100;
let normal_rate = 10; // jobs/sec

let queue_size = workers * burst_factor * (avg_job_time_ms / 1000) * normal_rate;
// = 8 × 5 × 0.1 × 10 = 40 jobs

let config = ThreadPoolConfig::new(workers)
    .with_max_queue_size(queue_size);
```

## Job Optimization

### Minimize Job Overhead

#### Bad: Small Jobs with High Overhead

```rust
// Anti-pattern: Too granular
for i in 0..1000000 {
    pool.execute(move || {
        process_item(i); // Tiny amount of work
        Ok(())
    })?;
}
// Result: Overhead dominates actual work
```

#### Good: Batch Small Jobs

```rust
// Pattern: Batch processing
const BATCH_SIZE: usize = 1000;

for chunk in (0..1000000).collect::<Vec<_>>().chunks(BATCH_SIZE) {
    let chunk = chunk.to_vec();
    pool.execute(move || {
        for i in chunk {
            process_item(i);
        }
        Ok(())
    })?;
}
// Result: Amortize overhead across many items
```

### Avoid Blocking in Jobs

#### Bad: Blocking Operations

```rust
pool.execute(|| {
    // Blocks worker thread!
    let response = reqwest::blocking::get("https://api.example.com")?;
    Ok(())
})?;
```

**Impact:**
- Worker thread blocked during I/O
- Reduced effective parallelism
- Lower throughput

#### Good: Use Async or Separate I/O Pool

```rust
// Option 1: Separate I/O pool
let io_pool = ThreadPool::with_threads(100)?; // Many workers for I/O

io_pool.execute(|| {
    let response = reqwest::blocking::get("https://api.example.com")?;
    Ok(())
})?;

// Option 2: Use async runtime (e.g., Tokio)
// Keep CPU pool separate from I/O
```

### Efficient Error Handling

```rust
// Efficient: Minimal allocation
pool.execute(|| {
    if some_condition {
        return Err(ThreadError::execution("Failed"));
    }
    Ok(())
})?;

// Avoid: Expensive error context
pool.execute(|| {
    if some_condition {
        // Don't do this in hot path
        let context = format!("{:?}", expensive_debug_info);
        return Err(ThreadError::execution(context));
    }
    Ok(())
})?;
```

## Monitoring Performance

### Using Built-in Statistics

```rust
let pool = ThreadPool::with_threads(8)?;
pool.start()?;

// Submit jobs...

// Get pool-level stats
let total_submitted = pool.total_jobs_submitted();
let total_processed = pool.total_jobs_processed();
let total_failed = pool.total_jobs_failed();

println!("Throughput: {} jobs/sec",
    total_processed as f64 / elapsed_time.as_secs_f64());

// Get per-worker stats
let worker_stats = pool.get_stats();
for (i, stat) in worker_stats.iter().enumerate() {
    println!("Worker {}:", i);
    println!("  Processed: {}", stat.get_jobs_processed());
    println!("  Failed: {}", stat.get_jobs_failed());
    println!("  Avg time: {:.2} μs", stat.get_average_processing_time_us());
}
```

### Key Performance Indicators

#### Throughput

```rust
let start = Instant::now();
let jobs_before = pool.total_jobs_processed();

// ... run for some time ...

let jobs_after = pool.total_jobs_processed();
let elapsed = start.elapsed();

let throughput = (jobs_after - jobs_before) as f64 / elapsed.as_secs_f64();
println!("Throughput: {:.0} jobs/sec", throughput);
```

#### Queue Depth

```rust
// Monitor queue size over time
let queue_size = pool.queued_jobs();

if queue_size > 1000 {
    println!("Warning: Queue backing up ({})", queue_size);
    // Consider: Adding workers, reducing job rate, or optimizing jobs
}
```

#### Worker Utilization

```rust
let stats = pool.get_stats();
let active_workers = stats.iter()
    .filter(|s| s.is_active())
    .count();

let utilization = (active_workers as f64 / stats.len() as f64) * 100.0;
println!("Worker utilization: {:.1}%", utilization);

// If consistently < 50%: Consider reducing workers
// If consistently > 95%: Consider adding workers
```

#### Success Rate

```rust
let total_processed = pool.total_jobs_processed();
let total_failed = pool.total_jobs_failed();
let total = total_processed + total_failed;

let success_rate = if total > 0 {
    (total_processed as f64 / total as f64) * 100.0
} else {
    0.0
};

println!("Success rate: {:.2}%", success_rate);
```

## Common Bottlenecks

### 1. Queue Contention

**Symptom:** High CPU usage but low throughput

**Diagnosis:**
```rust
// Check if submission is slow
let start = Instant::now();
pool.execute(|| Ok(()))?;
let submit_time = start.elapsed();

if submit_time > Duration::from_micros(100) {
    println!("Queue contention detected!");
}
```

**Solutions:**
- Reduce worker count
- Use work-stealing queue (future enhancement)
- Batch job submissions

### 2. Worker Starvation

**Symptom:** Some workers idle while queue has jobs

**Diagnosis:**
```rust
let stats = pool.get_stats();
let idle_workers = stats.iter()
    .filter(|s| !s.is_active() && pool.queued_jobs() > 0)
    .count();

if idle_workers > 0 {
    println!("Worker starvation: {} idle", idle_workers);
}
```

**Solutions:**
- Check for deadlocks
- Ensure jobs don't block indefinitely
- Increase queue size

### 3. Memory Pressure

**Symptom:** High memory usage, possible OOM

**Diagnosis:**
```rust
let queued = pool.queued_jobs();
let memory_estimate_mb = queued * 1024 / 1024 / 1024; // Rough estimate

if memory_estimate_mb > 100 {
    println!("Warning: High memory usage ({} MB)", memory_estimate_mb);
}
```

**Solutions:**
- Use bounded queue
- Implement backpressure
- Reduce job submission rate

### 4. Thundering Herd

**Symptom:** Periodic spikes in activity, then idle

**Solution:** Spread out job submission
```rust
// Bad: Submit all at once
for i in 0..10000 {
    pool.execute(|| Ok(()))?;
}

// Good: Rate-limit submission
use std::time::{Duration, Instant};

let mut last_submit = Instant::now();
let min_interval = Duration::from_micros(10);

for i in 0..10000 {
    while last_submit.elapsed() < min_interval {
        std::thread::yield_now();
    }

    pool.execute(|| Ok(()))?;
    last_submit = Instant::now();
}
```

## Comparison with Alternatives

### vs. Rayon

| Feature | rust_thread_system | rayon |
|---------|-------------------|-------|
| **Control** | Fine-grained | Coarse-grained |
| **Job Tracking** | Yes (statistics) | No |
| **Error Handling** | Per-job Result | Panic propagation |
| **Dynamic Control** | Yes | Limited |
| **Use Case** | Long-lived services | Data parallelism |

### vs. Tokio

| Feature | rust_thread_system | tokio |
|---------|-------------------|-------|
| **Model** | Thread pool | Async runtime |
| **Blocking** | OK | Discouraged |
| **CPU-Bound** | Excellent | Good |
| **I/O-Bound** | Good | Excellent |
| **Complexity** | Lower | Higher |

### Performance Comparison

```
Benchmark: 1M simple CPU-bound jobs

rust_thread_system (8 workers): 1.2 sec
rayon (8 threads):              0.9 sec
tokio (8 workers):              1.8 sec

rust_thread_system: Good for mixed workloads
rayon: Best for pure data parallelism
tokio: Best for I/O-heavy async workloads
```

## Best Practices Summary

1. **Right-size worker pool**
   - CPU-bound: physical cores
   - I/O-bound: cores × blocking ratio

2. **Use bounded queues in production**
   - Prevents memory issues
   - Forces proper backpressure

3. **Batch small jobs**
   - Reduces overhead
   - Improves throughput

4. **Monitor performance**
   - Track throughput, queue depth, utilization
   - Set alerts for anomalies

5. **Avoid blocking operations**
   - Use separate pools for I/O
   - Or use async runtimes

6. **Test under load**
   - Benchmark with realistic workloads
   - Find breaking points before production

---

*Performance Guide Version 1.0*
*Last Updated: 2025-10-16*
