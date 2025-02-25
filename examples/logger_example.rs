//! Example demonstrating the fixed logger implementation
//!
//! This example shows how to use the fixed logger module with all backends.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::path::PathBuf;
use std::time::Duration;

use log::Level;
use rust_thread_system::logger::{Logger, LoggerConfig};

fn main() -> rust_thread_system::error::Result<()> {
    println!("Starting logger example...");
    
    // Get the logger instance
    let logger = Logger::instance();
    
    // Configure the logger
    let config = LoggerConfig {
        app_name: "logger_example".to_string(),
        max_file_size: 1024 * 1024, // 1 MB
        use_backup: true,
        check_interval: Duration::from_millis(50),
        log_dir: PathBuf::from("logs"),
        file_level: Some(Level::Debug),
        console_level: Some(Level::Info),
        callback_level: Some(Level::Warn),
        ..Default::default()
    };
    
    logger.configure(config);
    
    // Setup callback counter
    let callback_counter = Arc::new(AtomicUsize::new(0));
    let callback_counter_clone = callback_counter.clone();
    
    // Set up callback
    logger.set_callback(move |level, timestamp, message| {
        println!("CALLBACK: [{}][{}] {}", timestamp, level, message);
        callback_counter_clone.fetch_add(1, Ordering::SeqCst);
    });
    
    // Start the logger
    logger.start()?;
    
    // Log messages at different levels
    logger.trace("This is a trace message - should only go to file")?;
    logger.debug("This is a debug message - should only go to file")?;
    logger.info("This is an info message - should go to console and file")?;
    logger.warn("This is a warning message - should go to all backends")?;
    logger.error("This is an error message - should go to all backends")?;
    
    // Log some messages using the macros
    let _ = rust_thread_system::log_info!("Info message using macro");
    let _ = rust_thread_system::log_warn!("Warning message using macro");
    let _ = rust_thread_system::log_error!("Error message using macro");
    
    // Wait for logs to be processed
    println!("Waiting for logs to be processed...");
    std::thread::sleep(Duration::from_millis(500));
    
    // Check callback count
    let count = callback_counter.load(Ordering::SeqCst);
    println!("Callback was called {} times", count);
    
    // Stop the logger
    logger.stop();
    println!("Logger stopped");
    
    // Try logging after the logger is stopped (should fail)
    match logger.info("This message should not be logged") {
        Ok(_) => println!("Unexpected: Message was logged after stopping"),
        Err(e) => println!("Expected error: {}", e),
    }
    
    println!("Logger example completed successfully!");
    Ok(())
}