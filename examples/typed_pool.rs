//! Example: Using TypedThreadPool with per-type routing
//!
//! This example demonstrates how to use TypedThreadPool for routing jobs
//! to dedicated queues based on their type.
//!
//! Run with: `cargo run --example typed_pool`

use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() -> Result<()> {
    println!("=== TypedThreadPool Example ===\n");

    // Create a typed pool with per-type worker configuration
    let config = TypedPoolConfig::<DefaultJobType>::new()
        .workers_for(DefaultJobType::Critical, 2)
        .workers_for(DefaultJobType::Compute, 4)
        .workers_for(DefaultJobType::Io, 8)
        .workers_for(DefaultJobType::Background, 1)
        .type_priority(vec![
            DefaultJobType::Critical,
            DefaultJobType::Io,
            DefaultJobType::Compute,
            DefaultJobType::Background,
        ]);

    println!("Configuration:");
    println!(
        "  Critical workers: {}",
        config.get_workers_for(DefaultJobType::Critical)
    );
    println!(
        "  Compute workers: {}",
        config.get_workers_for(DefaultJobType::Compute)
    );
    println!(
        "  IO workers: {}",
        config.get_workers_for(DefaultJobType::Io)
    );
    println!(
        "  Background workers: {}",
        config.get_workers_for(DefaultJobType::Background)
    );
    println!("  Total workers: {}", config.total_workers());
    println!();

    let pool = TypedThreadPool::new(config)?;
    pool.start()?;

    // Counters for each job type
    let critical_count = Arc::new(AtomicUsize::new(0));
    let compute_count = Arc::new(AtomicUsize::new(0));
    let io_count = Arc::new(AtomicUsize::new(0));
    let background_count = Arc::new(AtomicUsize::new(0));

    println!("Submitting jobs...\n");

    // Submit jobs of different types
    for i in 0..5 {
        // Critical jobs - highest priority
        let counter = Arc::clone(&critical_count);
        pool.execute_typed(DefaultJobType::Critical, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            println!("Critical job {} completed", i);
            Ok(())
        })?;
    }

    for i in 0..20 {
        // IO jobs - high concurrency
        let counter = Arc::clone(&io_count);
        pool.execute_typed(DefaultJobType::Io, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            // Simulate IO operation
            thread::sleep(Duration::from_millis(10));
            println!("IO job {} completed", i);
            Ok(())
        })?;
    }

    for i in 0..10 {
        // Compute jobs - CPU-bound
        let counter = Arc::clone(&compute_count);
        pool.execute_typed(DefaultJobType::Compute, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            // Simulate CPU work
            let _sum: u64 = (0..10000).sum();
            println!("Compute job {} completed", i);
            Ok(())
        })?;
    }

    for i in 0..3 {
        // Background jobs - lowest priority
        let counter = Arc::clone(&background_count);
        pool.execute_typed(DefaultJobType::Background, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            println!("Background job {} completed", i);
            Ok(())
        })?;
    }

    // Wait for jobs to complete
    thread::sleep(Duration::from_secs(1));

    // Print statistics
    println!("\n=== Statistics ===\n");

    println!("Jobs completed:");
    println!("  Critical: {}", critical_count.load(Ordering::Relaxed));
    println!("  IO: {}", io_count.load(Ordering::Relaxed));
    println!("  Compute: {}", compute_count.load(Ordering::Relaxed));
    println!("  Background: {}", background_count.load(Ordering::Relaxed));

    println!("\nPer-type statistics from pool:");

    for job_type in [
        DefaultJobType::Critical,
        DefaultJobType::Io,
        DefaultJobType::Compute,
        DefaultJobType::Background,
    ] {
        if let Some(stats) = pool.type_stats(job_type) {
            println!("  {:?}:", job_type);
            println!("    Submitted: {}", stats.jobs_submitted);
            println!("    Completed: {}", stats.jobs_completed);
            println!("    Failed: {}", stats.jobs_failed);
            println!("    Avg latency: {:?}", stats.avg_latency);
            println!("    Max latency: {:?}", stats.max_latency);
        }
    }

    println!("\nTotal jobs submitted: {}", pool.total_jobs_submitted());
    println!("Total queue depth: {}", pool.total_queue_depth());

    // Shutdown
    println!("\nShutting down...");
    pool.shutdown()?;
    println!("Done!");

    Ok(())
}
