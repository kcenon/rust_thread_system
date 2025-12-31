//! Bounded queue example
//!
//! Demonstrates queue capacity limits and backpressure handling.
//!
//! Run with: cargo run --example bounded_queue

use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() -> Result<()> {
    println!("=== Rust Thread System - Bounded Queue Example ===\n");

    // Create a thread pool with a bounded queue
    let config = ThreadPoolConfig::new(2)
        .with_max_queue_size(5)
        .with_thread_name_prefix("bounded-worker");

    let pool = ThreadPool::with_config(config)?;

    println!("1. Configuration:");
    println!("   Worker threads: {}", pool.num_threads());
    println!("   Maximum queue size: 5");

    pool.start()?;

    println!("\n2. Submitting long-running jobs to fill the queue:");

    let jobs_accepted = Arc::new(AtomicUsize::new(0));
    let jobs_rejected = Arc::new(AtomicUsize::new(0));

    // Try to submit many jobs
    for i in 0..20 {
        let result = pool.execute(move || {
            println!("  Job {} is executing", i);
            thread::sleep(Duration::from_millis(200));
            Ok(())
        });

        match result {
            Ok(()) => {
                jobs_accepted.fetch_add(1, Ordering::Relaxed);
                println!("  Job {} accepted", i);
            }
            Err(ThreadError::ShuttingDown { .. }) => {
                jobs_rejected.fetch_add(1, Ordering::Relaxed);
                println!("  Job {} rejected (queue full)", i);
            }
            Err(e) => {
                println!("  Job {} error: {}", i, e);
            }
        }

        // Small delay between submissions
        thread::sleep(Duration::from_millis(10));
    }

    println!(
        "\n3. Job submission results: {} accepted, {} rejected",
        jobs_accepted.load(Ordering::Relaxed),
        jobs_rejected.load(Ordering::Relaxed)
    );

    // Wait for jobs to complete
    println!("\n4. Waiting for jobs to complete...");
    thread::sleep(Duration::from_millis(2000));

    println!("\n5. Statistics:");
    println!("   Total submitted: {}", pool.total_jobs_submitted());
    println!("   Total processed: {}", pool.total_jobs_processed());

    println!("\n6. Trying to submit more jobs now that queue has space:");
    for i in 0..5 {
        match pool.execute(move || {
            println!("  Late job {} executing", i);
            Ok(())
        }) {
            Ok(()) => println!("  Late job {} accepted", i),
            Err(e) => println!("  Late job {} error: {}", i, e),
        }
    }

    thread::sleep(Duration::from_millis(300));

    println!("\n7. Final statistics:");
    println!("   Total submitted: {}", pool.total_jobs_submitted());
    println!("   Total processed: {}", pool.total_jobs_processed());

    pool.shutdown()?;

    println!("\n=== Example completed successfully! ===");
    println!("Note: Bounded queues help prevent memory exhaustion under heavy load");

    Ok(())
}
