//! Basic thread pool usage example
//!
//! Demonstrates thread pool creation, job submission, and statistics tracking.
//!
//! Run with: cargo run --example basic_usage

use rust_thread_system::prelude::*;
use std::thread;
use std::time::Duration;

fn main() -> Result<()> {
    println!("=== Rust Thread System - Basic Usage Example ===\n");

    // Create a thread pool with 4 worker threads
    let mut pool = ThreadPool::with_threads(4)?;

    println!(
        "1. Starting thread pool with {} threads",
        pool.num_threads()
    );
    pool.start()?;

    println!("\n2. Submitting simple jobs:");

    // Submit some simple jobs using closures
    for i in 0..10 {
        pool.execute(move || {
            println!(
                "  Job {} executing on thread {:?}",
                i,
                thread::current().id()
            );
            thread::sleep(Duration::from_millis(50));
            Ok(())
        })?;
    }

    println!("   Submitted 10 jobs");

    // Wait for jobs to complete
    thread::sleep(Duration::from_millis(200));

    println!("\n3. Job statistics:");
    println!("   Total jobs submitted: {}", pool.total_jobs_submitted());
    println!("   Total jobs processed: {}", pool.total_jobs_processed());
    println!("   Total jobs failed: {}", pool.total_jobs_failed());

    // Get per-worker statistics
    println!("\n4. Per-worker statistics:");
    let stats = pool.get_stats();
    for (i, stat) in stats.iter().enumerate() {
        println!(
            "   Worker {}: {} processed, {} failed, avg time: {:.2}μs",
            i,
            stat.get_jobs_processed(),
            stat.get_jobs_failed(),
            stat.get_average_processing_time_us()
        );
    }

    // Submit more jobs
    println!("\n5. Submitting more jobs:");
    for i in 10..20 {
        pool.execute(move || {
            println!("  Job {} executing", i);
            Ok(())
        })?;
    }

    // Wait for completion
    thread::sleep(Duration::from_millis(100));

    println!("\n6. Final statistics:");
    println!("   Total jobs submitted: {}", pool.total_jobs_submitted());
    println!("   Total jobs processed: {}", pool.total_jobs_processed());

    // Graceful shutdown
    println!("\n7. Shutting down thread pool...");
    pool.shutdown()?;

    println!("\n=== Example completed successfully! ===");

    Ok(())
}
