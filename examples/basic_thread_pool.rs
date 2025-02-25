//! # Basic Thread Pool Example
//!
//! This example demonstrates how to use the basic thread pool to run concurrent jobs.
//! It also shows how to properly initialize and use the logger.

use rust_thread_system::ThreadPool;
use rust_thread_system::job::CallbackJob;
use rust_thread_system::logger::{Logger, LoggerConfig};
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::PathBuf;
use log::Level;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the logger first
    setup_logger()?;
    
    let _ = rust_thread_system::log_info!("=== Basic Thread Pool Example ===");
    
    // Create a thread pool with 4 workers
    let pool = ThreadPool::with_workers(4);
    let _ = rust_thread_system::log_info!("Created thread pool with 4 workers");
    
    // Start the thread pool
    pool.start()?;
    let _ = rust_thread_system::log_info!("Thread pool started");
    
    // Create a counter to track job completions
    let counter = Arc::new(AtomicUsize::new(0));
    let total_jobs = 10;
    
    let _ = rust_thread_system::log_info!("Submitting {} jobs to the thread pool...", total_jobs);
    
    // Submit jobs to the thread pool
    for i in 0..total_jobs {
        let counter_clone = Arc::clone(&counter);
        
        // Create a job with the CallbackJob utility
        let job = CallbackJob::new(move || {
            let _ = rust_thread_system::log_info!("Job {} is executing", i);
            
            // Simulate some work
            let work_time = 100 + (i * 50) % 400;
            std::thread::sleep(Duration::from_millis(work_time as u64));
            
            // Increment the completion counter
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let _ = rust_thread_system::log_info!("Job {} completed", i);
            
            Ok(())
        }, format!("job_{}", i));
        
        // Submit the job to the thread pool
        pool.submit(job)?;
        let _ = rust_thread_system::log_info!("Submitted job {}", i);
    }
    
    let _ = rust_thread_system::log_info!("Waiting for jobs to complete...");
    
    // Check job completion in a loop
    for i in 1..=10 {
        let completed = counter.load(Ordering::SeqCst);
        let _ = rust_thread_system::log_info!("Progress: {}/{} jobs completed", completed, total_jobs);
        
        if completed >= total_jobs {
            let _ = rust_thread_system::log_info!("All jobs completed!");
            break;
        }
        
        if i == 10 {
            let _ = rust_thread_system::log_warn!("Timeout waiting for jobs to complete.");
            break;
        }
        
        // Wait a bit before checking again
        std::thread::sleep(Duration::from_millis(200));
    }
    
    // Stop the thread pool (gracefully)
    let _ = rust_thread_system::log_info!("Stopping thread pool (waiting for jobs to complete)...");
    pool.stop(true);
    
    let _ = rust_thread_system::log_info!("=== Example completed ===");
    
    // Stop logger before exiting
    Logger::instance().stop();
    
    Ok(())
}

/// Set up the logger with a reasonable configuration
fn setup_logger() -> Result<(), Box<dyn std::error::Error>> {
    // Create a logger configuration
    let config = LoggerConfig {
        app_name: "basic_thread_pool_example".to_string(),
        max_file_size: 1024 * 1024, // 1 MB
        use_backup: true,
        check_interval: Duration::from_millis(50),
        log_dir: PathBuf::from("logs"),
        file_level: Some(Level::Info),
        console_level: Some(Level::Info),
        callback_level: None,
    };
    
    // Configure and start the logger
    let logger = Logger::instance();
    logger.configure(config);
    logger.start()?;
    
    // Optionally initialize the global logger for the log crate
    logger.init_global_logger()?;
    
    Ok(())
}