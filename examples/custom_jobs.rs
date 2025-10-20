//! Custom job types example
//!
//! Demonstrates implementing custom Job types and named closure jobs.
//!
//! Run with: cargo run --example custom_jobs

use rust_thread_system::prelude::*;
use std::thread;
use std::time::Duration;

/// A custom job that processes data
struct DataProcessingJob {
    id: usize,
    data: Vec<u32>,
}

impl DataProcessingJob {
    fn new(id: usize, data: Vec<u32>) -> Self {
        Self { id, data }
    }
}

impl Job for DataProcessingJob {
    fn execute(&mut self) -> Result<()> {
        println!(
            "Job {}: Processing {} items on thread {:?}",
            self.id,
            self.data.len(),
            thread::current().id()
        );

        // Simulate processing
        let sum: u32 = self.data.iter().sum();
        let avg = sum as f64 / self.data.len() as f64;

        println!("Job {}: Sum = {}, Average = {:.2}", self.id, sum, avg);

        thread::sleep(Duration::from_millis(100));

        Ok(())
    }

    fn job_type(&self) -> &str {
        "DataProcessingJob"
    }
}

/// A job that can fail
struct FallibleJob {
    id: usize,
    should_fail: bool,
}

impl Job for FallibleJob {
    fn execute(&mut self) -> Result<()> {
        println!("FallibleJob {}: Starting", self.id);

        if self.should_fail {
            return Err(ThreadError::execution(
                format!("job_{}", self.id),
                format!("Job {} intentionally failed", self.id),
            ));
        }

        println!("FallibleJob {}: Completed successfully", self.id);
        Ok(())
    }

    fn job_type(&self) -> &str {
        "FallibleJob"
    }
}

/// A named closure job
fn create_named_job(name: &str, value: i32) -> ClosureJob<impl FnOnce() -> Result<()> + Send> {
    let job_name = name.to_string();
    ClosureJob::with_name(
        move || {
            println!("{}: Processing value {}", job_name, value);
            thread::sleep(Duration::from_millis(50));
            Ok(())
        },
        name,
    )
}

fn main() -> Result<()> {
    println!("=== Rust Thread System - Custom Jobs Example ===\n");

    let mut pool = ThreadPool::with_threads(4)?;
    pool.start()?;

    println!("1. Submitting custom data processing jobs:");
    for i in 0..5 {
        let data: Vec<u32> = (0..10).map(|x| x * (i as u32 + 1)).collect();
        pool.submit(DataProcessingJob::new(i, data))?;
    }

    thread::sleep(Duration::from_millis(600));

    println!("\n2. Submitting fallible jobs:");
    for i in 0..8 {
        pool.submit(FallibleJob {
            id: i,
            should_fail: i % 3 == 0, // Every third job fails
        })?;
    }

    thread::sleep(Duration::from_millis(200));

    println!("\n3. Submitting named closure jobs:");
    for i in 0..5 {
        pool.submit(create_named_job(&format!("Calculator-{}", i), i * 10))?;
    }

    thread::sleep(Duration::from_millis(300));

    println!("\n4. Statistics:");
    println!("   Total jobs submitted: {}", pool.total_jobs_submitted());
    println!("   Total jobs processed: {}", pool.total_jobs_processed());
    println!("   Total jobs failed: {}", pool.total_jobs_failed());

    println!("\n5. Per-worker breakdown:");
    for (i, stat) in pool.get_stats().iter().enumerate() {
        println!(
            "   Worker {}: {} processed, {} failed",
            i,
            stat.get_jobs_processed(),
            stat.get_jobs_failed()
        );
    }

    pool.shutdown()?;

    println!("\n=== Example completed successfully! ===");

    Ok(())
}
