//! # Priority Thread Pool Example
//!
//! This example demonstrates how to use the priority thread pool to run jobs with different priority levels.

use rust_thread_system::{PriorityThreadPool, JobPriority, logger::{Logger, LoggerConfig}};
use rust_thread_system::job::CallbackJob;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::PathBuf;
use log::Level;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the logger
    setup_logger()?;

    let _ = rust_thread_system::log_info!("=== Priority Thread Pool Example ===");
    
    // Create a priority thread pool with 3 workers
    let pool = PriorityThreadPool::with_workers(3);
    let _ = rust_thread_system::log_info!("Created priority thread pool with 3 workers");
    
    // Start the thread pool
    pool.start()?;
    let _ = rust_thread_system::log_info!("Priority thread pool started");
    
    // Create counters to track job completions by priority
    let high_counter = Arc::new(AtomicUsize::new(0));
    let medium_counter = Arc::new(AtomicUsize::new(0));
    let low_counter = Arc::new(AtomicUsize::new(0));
    
    // Submit jobs with different priorities
    let _ = rust_thread_system::log_info!("Submitting jobs with different priorities...");
    
    // Submit low priority jobs (these will execute last)
    let _ = rust_thread_system::log_info!("Submitting 5 LOW priority jobs...");
    for i in 0..5 {
        let counter_clone = Arc::clone(&low_counter);
        let job = CallbackJob::new(move || {
            let _ = rust_thread_system::log_info!("LOW priority job {} is executing", i);
            std::thread::sleep(Duration::from_millis(100));
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let _ = rust_thread_system::log_info!("LOW priority job {} completed", i);
            Ok(())
        }, format!("low_job_{}", i));
        
        pool.submit_with_priority(job, JobPriority::Low)?;
    }
    
    // Submit medium priority jobs (these will execute second)
    let _ = rust_thread_system::log_info!("Submitting 5 MEDIUM priority jobs...");
    for i in 0..5 {
        let counter_clone = Arc::clone(&medium_counter);
        let job = CallbackJob::new(move || {
            let _ = rust_thread_system::log_info!("MEDIUM priority job {} is executing", i);
            std::thread::sleep(Duration::from_millis(100));
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let _ = rust_thread_system::log_info!("MEDIUM priority job {} completed", i);
            Ok(())
        }, format!("medium_job_{}", i));
        
        pool.submit_with_priority(job, JobPriority::Normal)?;
    }
    
    // Submit high priority jobs (these will execute first)
    let _ = rust_thread_system::log_info!("Submitting 5 HIGH priority jobs...");
    for i in 0..5 {
        let counter_clone = Arc::clone(&high_counter);
        let job = CallbackJob::new(move || {
            let _ = rust_thread_system::log_info!("HIGH priority job {} is executing", i);
            std::thread::sleep(Duration::from_millis(100));
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let _ = rust_thread_system::log_info!("HIGH priority job {} completed", i);
            Ok(())
        }, format!("high_job_{}", i));
        
        pool.submit_with_priority(job, JobPriority::High)?;
    }
    
    let _ = rust_thread_system::log_info!("Waiting for jobs to complete...");
    let _ = rust_thread_system::log_info!("Notice that HIGH priority jobs generally execute before MEDIUM priority jobs,");
    let _ = rust_thread_system::log_info!("and MEDIUM priority jobs generally execute before LOW priority jobs.");
    
    // Wait for jobs to complete
    for _ in 0..10 {
        let high_completed = high_counter.load(Ordering::SeqCst);
        let medium_completed = medium_counter.load(Ordering::SeqCst);
        let low_completed = low_counter.load(Ordering::SeqCst);
        
        let _ = rust_thread_system::log_info!("Progress: HIGH: {}/5, MEDIUM: {}/5, LOW: {}/5", 
                high_completed, medium_completed, low_completed);
        
        if high_completed + medium_completed + low_completed >= 15 {
            let _ = rust_thread_system::log_info!("All jobs completed!");
            break;
        }
        
        std::thread::sleep(Duration::from_millis(200));
    }
    
    // Stop the thread pool
    let _ = rust_thread_system::log_info!("Stopping priority thread pool...");
    pool.stop(true);
    let _ = rust_thread_system::log_info!("Priority thread pool stopped");
    
    let _ = rust_thread_system::log_info!("=== Example completed ===");
    
    // Stop the logger before exiting
    Logger::instance().stop();
    Ok(())
}

/// Set up the logger with a reasonable configuration
fn setup_logger() -> Result<(), Box<dyn std::error::Error>> {
    // Create a logger configuration
    let config = LoggerConfig {
        app_name: "priority_thread_pool_example".to_string(),
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
    
    // Initialize the global logger for the log crate
    logger.init_global_logger()?;
    
    Ok(())
}