//! Advanced features demonstration
//!
//! This example demonstrates the new features added to the thread pool:
//! - Pool statistics snapshot
//! - Builder pattern
//! - Job result return
//! - Backpressure strategies
//! - Named jobs

use rust_thread_system::prelude::*;
use std::thread;
use std::time::Duration;

fn main() -> Result<()> {
    println!("=== Advanced Thread Pool Features Demo ===\n");

    // Feature 1: Builder Pattern
    println!("1. Using Builder Pattern:");
    let mut pool = ThreadPool::builder()
        .num_threads(4)
        .max_queue_size(100)
        .thread_name_prefix("demo-worker")
        .build_and_start()?;
    println!("   ✓ Created pool with 4 workers using builder\n");

    // Feature 2: Named Jobs
    println!("2. Named Jobs:");
    pool.execute_named("data-processing", || {
        println!("   → Executing data-processing job");
        thread::sleep(Duration::from_millis(100));
        Ok(())
    })?;

    pool.execute_named("image-resize", || {
        println!("   → Executing image-resize job");
        thread::sleep(Duration::from_millis(100));
        Ok(())
    })?;
    println!("   ✓ Submitted named jobs\n");

    // Feature 3: Job Results
    println!("3. Job Results:");
    let result_handle = pool.execute_with_result(|| {
        println!("   → Computing factorial of 10");
        let factorial: u64 = (1..=10).product();
        Ok(factorial)
    })?;

    // Wait for result with timeout
    match result_handle.wait_timeout(Duration::from_secs(2))? {
        Some(result) => println!("   ✓ Got result: 10! = {}", result),
        None => println!("   ✗ Timeout waiting for result"),
    }
    println!();

    // Submit multiple jobs with results
    println!("4. Multiple Job Results:");
    let mut handles = vec![];
    for i in 1..=5 {
        let handle = pool.execute_with_result(move || {
            thread::sleep(Duration::from_millis(50));
            Ok(i * i)
        })?;
        handles.push(handle);
    }

    println!("   Waiting for {} jobs to complete...", handles.len());
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.wait()? {
            result => println!("   Job {}: result = {}", i + 1, result),
        }
    }
    println!();

    // Feature 4: Pool Statistics Snapshot
    println!("5. Pool Statistics:");

    // Submit some jobs
    for i in 0..10 {
        pool.execute_named(&format!("batch-job-{}", i), move || {
            thread::sleep(Duration::from_millis(50));
            if i % 3 == 0 {
                Err(ThreadError::other("Simulated error"))
            } else {
                Ok(())
            }
        })?;
    }

    // Wait a bit for jobs to process
    thread::sleep(Duration::from_millis(300));

    // Get comprehensive statistics with a single call
    let stats = pool.get_pool_stats();
    println!("   Jobs submitted:  {}", stats.total_jobs_submitted);
    println!("   Jobs processed:  {}", stats.total_jobs_processed);
    println!("   Jobs failed:     {}", stats.total_jobs_failed);
    println!("   Success rate:    {:.1}%", stats.success_rate());
    println!("   Queue size:      {}", stats.current_queue_size);
    println!("   Workers:         {}", stats.num_workers);
    println!("   Avg time:        {:.2} μs", stats.average_processing_time_us());

    println!("\n   Per-worker statistics:");
    for (i, worker) in stats.worker_stats.iter().enumerate() {
        println!("   Worker {}: {} processed, {} failed",
                 i, worker.jobs_processed, worker.jobs_failed);
    }
    println!();

    // Feature 5: Backpressure Strategy
    println!("6. Backpressure Strategy:");
    let mut small_pool = ThreadPool::builder()
        .num_threads(1)
        .max_queue_size(3)  // Very small queue
        .build_and_start()?;

    // Try to fill the queue
    println!("   Filling small queue (size=3)...");
    for i in 0..5 {
        let strategy = BackpressureStrategy::RetryWithBackoff {
            max_retries: 3,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(200),
        };

        // Note: Due to FnOnce limitations, we show the strategy API
        // In real code, you might use a factory pattern
        match small_pool.execute(move || {
            println!("   → Job {} executing", i);
            thread::sleep(Duration::from_millis(100));
            Ok(())
        }) {
            Ok(()) => println!("   ✓ Job {} submitted", i),
            Err(e) => {
                println!("   ✗ Job {} failed: {}", i, e);
                println!("   (In production, use backpressure strategy)");
            }
        }
    }

    // Wait for small pool jobs
    thread::sleep(Duration::from_secs(1));
    small_pool.shutdown()?;
    println!("   ✓ Small pool shutdown\n");

    // Shutdown main pool
    println!("7. Graceful Shutdown:");
    println!("   Shutting down main pool...");
    let final_stats = pool.get_pool_stats();
    pool.shutdown()?;
    println!("   ✓ Pool shutdown gracefully");
    println!("   Final stats: {} total jobs processed", final_stats.total_jobs_processed);

    println!("\n=== Demo Complete ===");
    Ok(())
}
