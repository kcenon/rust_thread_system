//! Thread-safe logging system with multiple backends.
//!
//! This module provides a thread-safe logging system with support for multiple
//! backends (console, file, callback) and log levels.

use std::path::{PathBuf};
use std::io::Write;
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::thread;

use chrono::Local;
use crossbeam_channel::{Sender, Receiver};
use log::{Level, Log, Metadata, Record};

use crate::error::{Error, Result};

/// Static global logger wrapper for log crate integration
static LOGGER_WRAPPER: LoggerWrapper = LoggerWrapper;

/// Logger wrapper struct implementing the log::Log trait
#[derive(Debug)]
struct LoggerWrapper;

impl Log for LoggerWrapper {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        // We'll let the Logger instance do the filtering
        true
    }

    fn log(&self, record: &Record) {
        if let Some(logger) = Logger::try_instance() {
            let entry = LogEntry {
                message: format!("{}", record.args()),
                level: record.level(),
                timestamp: Local::now(),
            };
            
            // Try to log the entry, ignore errors here
            let _ = logger.log_entry(entry);
        }
    }

    fn flush(&self) {
        // Not much we can do here in a static context
    }
}

/// Log entry containing message and metadata
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Log message
    message: String,
    /// Log level
    level: Level,
    /// Timestamp for when the log was created
    timestamp: chrono::DateTime<chrono::Local>,
}

impl LogEntry {
    /// Create a new log entry
    pub fn new(message: impl Into<String>, level: Level) -> Self {
        Self {
            message: message.into(),
            level,
            timestamp: Local::now(),
        }
    }
    
    /// Format the log entry for output
    pub fn format(&self) -> String {
        format!(
            "[{}][{}] {}", 
            self.timestamp.format("%Y-%m-%d %H:%M:%S%.6f"),
            self.level,
            self.message
        )
    }
}

/// Callback type for custom log handling
pub type LogCallback = Arc<dyn Fn(Level, String, String) + Send + Sync + 'static>;

/// Logger configuration
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    /// Application name/title
    pub app_name: String,
    /// Maximum log file size in bytes
    pub max_file_size: u64,
    /// Whether to use backup files when rotating logs
    pub use_backup: bool,
    /// Log check interval
    pub check_interval: Duration,
    /// Path to log directory
    pub log_dir: PathBuf,
    /// File log level
    pub file_level: Option<Level>,
    /// Console log level
    pub console_level: Option<Level>,
    /// Callback log level
    pub callback_level: Option<Level>,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            app_name: "application".to_string(),
            max_file_size: 10 * 1024 * 1024, // 10 MB
            use_backup: true,
            check_interval: Duration::from_millis(100),
            log_dir: PathBuf::from("logs"),
            file_level: Some(Level::Info),
            console_level: Some(Level::Info),
            callback_level: None,
        }
    }
}

/// Worker state containing thread handle and stop flag
struct LogWorker {
    /// Thread handle
    handle: Option<thread::JoinHandle<()>>,
    /// Flag to signal the thread to stop
    stop_flag: Arc<AtomicBool>,
}

impl LogWorker {
    /// Create a new worker
    fn new() -> Self {
        Self {
            handle: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }
    
    /// Stop the worker and wait for it to join
    fn stop(&mut self) {
        // Signal thread to stop
        self.stop_flag.store(true, Ordering::SeqCst);
        
        // Wait for thread to join if it exists
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
    
}

/// A thread-safe singleton logger
pub struct Logger {
    /// Logger configuration
    config: RwLock<LoggerConfig>,
    /// Log message channel sender
    sender: Mutex<Option<Sender<LogEntry>>>,
    /// Whether the logger is running
    running: AtomicBool,
    /// User-provided callback for log messages
    callback: Mutex<Option<LogCallback>>,
    /// Console log worker
    console_worker: Mutex<Option<LogWorker>>,
    /// File log worker
    file_worker: Mutex<Option<LogWorker>>,
    /// Callback log worker
    callback_worker: Mutex<Option<LogWorker>>,
}

// Static logger instance
static LOGGER_INSTANCE: OnceLock<Logger> = OnceLock::new();

impl Logger {
    /// Get the singleton logger instance
    pub fn instance() -> &'static Logger {
        LOGGER_INSTANCE.get_or_init(|| {
            Logger {
                config: RwLock::new(LoggerConfig::default()),
                sender: Mutex::new(None),
                running: AtomicBool::new(false),
                callback: Mutex::new(None),
                console_worker: Mutex::new(None),
                file_worker: Mutex::new(None),
                callback_worker: Mutex::new(None),
            }
        })
    }
    
    /// Try to get the singleton logger instance without initializing it if it doesn't exist
    pub fn try_instance() -> Option<&'static Logger> {
        LOGGER_INSTANCE.get()
    }
    
    /// Set the logger configuration
    pub fn configure(&self, config: LoggerConfig) {
        if let Ok(mut cfg) = self.config.write() {
            *cfg = config;
        }
    }
    
    /// Set the log callback
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(Level, String, String) + Send + Sync + 'static,
    {
        if let Ok(mut cb) = self.callback.lock() {
            *cb = Some(Arc::new(callback));
        }
    }
    
    /// Initialize the global logger for the log crate
    pub fn init_global_logger(&self) -> std::result::Result<(), log::SetLoggerError> {
        log::set_max_level(log::LevelFilter::Trace);
        log::set_logger(&LOGGER_WRAPPER)
    }
    
    /// Start the logger with a completely redesigned architecture
    pub fn start(&self) -> Result<()> {
        if self.is_running() {
            return Err(Error::logger_error("Logger is already running"));
        }
        
        // Create log directory if it doesn't exist
        let config = self.config.read().map_err(|e| 
            Error::lock_error(format!("Failed to lock logger config: {}", e))
        )?;
        
        // Create log directory with more verbose error handling
        match std::fs::create_dir_all(&config.log_dir) {
            Ok(_) => {
                // Verify that the directory is actually created and writable
                if !config.log_dir.exists() {
                    return Err(Error::logger_error(format!(
                        "Failed to create log directory at {:?}: directory doesn't exist after creation", 
                        config.log_dir
                    )));
                }
                
                // Test file creation to ensure we have write permission
                let test_file_path = config.log_dir.join(".write_test");
                let test_result = std::fs::File::create(&test_file_path);
                
                match test_result {
                    Ok(_) => {
                        // Successfully created test file, clean it up
                        let _ = std::fs::remove_file(test_file_path);
                    },
                    Err(e) => {
                        return Err(Error::logger_error(format!(
                            "Log directory exists but is not writable: {}", e
                        )));
                    }
                }
            },
            Err(e) => {
                return Err(Error::logger_error(format!(
                    "Failed to create log directory at {:?}: {}", 
                    config.log_dir, e
                )));
            }
        }
        
        // Print informative message about logger configuration
        println!("Logger configured with file path: {:?}", config.log_dir.join(format!("{}.log", config.app_name)));
        
        // CRITICAL ARCHITECTURAL CHANGE:
        // Create a single primary channel that receives ALL logs
        // Then use a dispatcher model to guarantee delivery to all backends
        let (primary_sender, primary_receiver) = crossbeam_channel::bounded::<LogEntry>(100_000);
        
        // Store primary sender for client code to use
        *self.sender.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock sender: {}", e))
        )? = Some(primary_sender.clone());
        
        // Set as running first to ensure messages can be sent while workers are starting
        self.running.store(true, Ordering::SeqCst);
        
        // Create a separate channel for each output destination
        let (console_sender, console_receiver) = crossbeam_channel::bounded::<LogEntry>(100_000);
        let (file_sender, file_receiver) = crossbeam_channel::bounded::<LogEntry>(100_000);
        let (callback_sender, callback_receiver) = crossbeam_channel::bounded::<LogEntry>(100_000);
        
        // Start a central dispatcher thread that reads from primary_receiver
        // and fans out to all enabled backends
        let stop_flag = Arc::new(AtomicBool::new(false));
        let dispatch_stop_flag = stop_flag.clone();
        let dispatch_stop_flag_for_error = stop_flag.clone();
        
        // Create a worker container for the dispatcher
        let mut dispatch_worker = LogWorker::new();
        dispatch_worker.stop_flag = stop_flag;
        
        // Create dispatcher thread that ensures ALL messages go to ALL backends
        let handle = match thread::Builder::new()
            .name("log_dispatcher".to_string())
            .spawn(move || {
                println!("Log dispatcher started");
                
                while !dispatch_stop_flag.load(Ordering::SeqCst) {
                    // Block waiting for the next message on the primary channel
                    match primary_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(entry) => {
                            // GUARANTEED DELIVERY: Send to ALL backends
                            // Even if one backend is slow, messages will queue up
                            // and won't be lost
                            let _ = console_sender.send(entry.clone());
                            let _ = file_sender.send(entry.clone());
                            let _ = callback_sender.send(entry);
                            
                            // Process any batched messages
                            while let Ok(next_entry) = primary_receiver.try_recv() {
                                let _ = console_sender.send(next_entry.clone());
                                let _ = file_sender.send(next_entry.clone());
                                let _ = callback_sender.send(next_entry);
                            }
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // Timeout is expected, just check if we should stop
                            continue;
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            // Primary channel disconnected, exit the worker
                            break;
                        }
                    }
                }
                
                println!("Log dispatcher stopping");
                // Final flush of any pending messages
                while let Ok(entry) = primary_receiver.try_recv() {
                    let _ = console_sender.send(entry.clone());
                    let _ = file_sender.send(entry.clone());
                    let _ = callback_sender.send(entry);
                }
                
                println!("Log dispatcher finished");
            }) {
                Ok(handle) => handle,
                Err(e) => {
                    dispatch_stop_flag_for_error.store(true, Ordering::SeqCst);
                    return Err(Error::thread_error(format!("Failed to start dispatcher: {}", e)));
                }
            };
            
        dispatch_worker.handle = Some(handle);
        
        // Start the individual output workers
        if let Err(e) = self.setup_console_worker(console_receiver) {
            eprintln!("Failed to setup console worker: {}", e);
            // Continue with other workers
        }
        if let Err(e) = self.setup_file_worker(file_receiver) {
            eprintln!("Failed to setup file worker: {}", e);
            // Continue with other workers
        }
        if let Err(e) = self.setup_callback_worker(callback_receiver) {
            eprintln!("Failed to setup callback worker: {}", e);
            // Continue with other workers
        }
        
        // Store the dispatcher worker
        *self.console_worker.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock console worker: {}", e))
        )? = Some(dispatch_worker);
        
        Ok(())
    }
    
    /// Stop the logger with improved clean shutdown process
    pub fn stop(&self) {
        // Explicitly use println since we're stopping the logger itself
        println!("Stopping logger...");
        
        // First set running to false to prevent new messages
        self.running.store(false, Ordering::SeqCst);
        
        // To ensure clean shutdown, we need to:
        // 1. First, let each backend worker know it should stop
        // 2. Let each one finish its queue of pending messages
        // 3. Stop the dispatcher last
        
        // Give worker threads time to process any pending logs
        std::thread::sleep(std::time::Duration::from_millis(200));
        
        // Stop all workers in proper order
        self.stop_workers();
        
        // Reset channels after threads have stopped
        if let Ok(mut sender_lock) = self.sender.lock() {
            *sender_lock = None;
        }
        
        println!("Logger stopped successfully");
    }
    
    /// Improved stop_workers method ensures proper shutdown order
    fn stop_workers(&self) {
        // Stop backend workers first (file, callback)
        // File worker
        if let Ok(mut worker) = self.file_worker.lock() {
            if let Some(worker) = worker.as_mut() {
                worker.stop();
            }
            *worker = None;
        }
        
        // Callback worker
        if let Ok(mut worker) = self.callback_worker.lock() {
            if let Some(worker) = worker.as_mut() {
                worker.stop();
            }
            *worker = None;
        }
        
        // Wait a little for backend workers to finish
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        // Console worker is typically faster so stop it after the others
        if let Ok(mut worker) = self.console_worker.lock() {
            if let Some(worker) = worker.as_mut() {
                worker.stop();
                // Wait for it to finish
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            *worker = None;
        }
    }
    
    
    /// Check if the logger is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    /// Set up the console log worker
    fn setup_console_worker(&self, receiver: Receiver<LogEntry>) -> Result<()> {
        // Get necessary values while holding the locks
        let console_level = {
            let config = self.config.read().map_err(|e| 
                Error::lock_error(format!("Failed to lock logger config: {}", e))
            )?;
            
            match config.console_level {
                Some(level) => level,
                None => return Ok(()), // Console logging disabled
            }
        };
        
        // We no longer need check_interval as we use fixed timeouts
        {
            let _config = self.config.read().map_err(|e| 
                Error::lock_error(format!("Failed to lock logger config: {}", e))
            )?;
            // Config is read only to maintain the lock pattern, but we don't need the interval
        };
        
        let mut worker = LogWorker::new();
        let stop_flag = worker.stop_flag.clone();
        let stop_flag_for_error = worker.stop_flag.clone();
        
        // Spawn the worker thread
        let handle = match thread::Builder::new()
            .name("console_logger".to_string())
            .spawn(move || {
                // Buffer no longer needed with direct processing
                
                // Keep running until signaled to stop
                while !stop_flag.load(Ordering::SeqCst) {
                    // Use timeout to regularly check stop flag
                    match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(entry) => {
                            if entry.level <= console_level {
                                // Directly print the entry
                                println!("{}", entry.format());
                                
                                // Process any batch of pending entries
                                let mut batch_count = 0;
                                while let Ok(next_entry) = receiver.try_recv() {
                                    if next_entry.level <= console_level {
                                        println!("{}", next_entry.format());
                                        batch_count += 1;
                                        if batch_count >= 20 {  // Process up to 20 entries at once
                                            // Force flush stdout after a batch
                                            let _ = std::io::stdout().flush();
                                            break;
                                        }
                                    }
                                }
                                
                                // Force flush stdout after processing all entries
                                let _ = std::io::stdout().flush();
                            }
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // Timeout is normal, continue
                            continue;
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            // Channel disconnected, exit the worker
                            println!("Console logger channel disconnected, exiting worker");
                            break;
                        }
                    }
                }
                
                // Process any remaining messages before shutting down
                println!("Processing remaining console logger messages before shutdown");
                while let Ok(entry) = receiver.try_recv() {
                    if entry.level <= console_level {
                        println!("{}", entry.format());
                    }
                }
                let _ = std::io::stdout().flush();
                
                // Clean up before terminating
                println!("Console log worker is shutting down");
            }) {
                Ok(handle) => handle,
                Err(e) => {
                    stop_flag_for_error.store(true, Ordering::SeqCst);
                    return Err(Error::thread_error(format!("Failed to start console worker: {}", e)));
                }
            };
        
        worker.handle = Some(handle);
        
        // Store the worker
        *self.console_worker.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock console worker: {}", e))
        )? = Some(worker);
        
        Ok(())
    }
    
    /// Set up the file log worker
    fn setup_file_worker(&self, receiver: Receiver<LogEntry>) -> Result<()> {
        // Get necessary values while holding the locks
        let (file_level, app_name, log_dir, max_file_size, use_backup) = {
            let config = self.config.read().map_err(|e| 
                Error::lock_error(format!("Failed to lock logger config: {}", e))
            )?;
            
            let level = match config.file_level {
                Some(level) => level,
                None => return Ok(()), // File logging disabled
            };
            
            (
                level,
                config.app_name.clone(),
                config.log_dir.clone(),
                config.max_file_size,
                config.use_backup
            )
        };
        
        let mut worker = LogWorker::new();
        let stop_flag = worker.stop_flag.clone();
        let stop_flag_for_error = worker.stop_flag.clone();
        
        let handle = match thread::Builder::new()
            .name("file_logger".to_string())
            .spawn(move || {
                // Buffer no longer needed with direct processing
                let mut current_size: u64 = 0;
                let log_path = log_dir.join(format!("{}.log", app_name));
                
                // Make sure the log directory exists before opening the file
                if let Err(e) = std::fs::create_dir_all(&log_dir) {
                    eprintln!("Failed to create log directory: {}", e);
                    // Try to create it anyway in case the error was transient
                }
                
                // Helper function for opening a log file with retries
                let open_log_file_with_retry = |path: &PathBuf, retries: usize| -> Option<std::fs::File> {
                    for attempt in 0..=retries {
                        match OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path) 
                        {
                            Ok(file) => return Some(file),
                            Err(e) => {
                                if attempt < retries {
                                    eprintln!("Failed to open log file (attempt {}/{}): {}", 
                                        attempt + 1, retries, e);
                                    // Recreate directory and pause before retry
                                    let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
                                    std::thread::sleep(std::time::Duration::from_millis(50 * (attempt as u64 + 1)));
                                } else {
                                    eprintln!("Final attempt to open log file failed: {}", e);
                                }
                            }
                        }
                    }
                    None
                };
                
                // Create or open log file with multiple retries
                let mut file = match open_log_file_with_retry(&log_path, 3) {
                    Some(file) => file,
                    None => {
                        eprintln!("Failed to open log file after multiple retries: {}", log_path.display());
                        return;
                    }
                };
                
                // Get current file size
                if let Ok(metadata) = std::fs::metadata(&log_path) {
                    current_size = metadata.len();
                }
                
                // Write a startup marker to the log file
                let startup_message = format!("[{}] === Logger started ===\n", 
                    Local::now().format("%Y-%m-%d %H:%M:%S%.6f"));
                if let Err(e) = file.write_all(startup_message.as_bytes()) {
                    eprintln!("Failed to write startup marker to log file: {}", e);
                } else {
                    // Immediately flush
                    let _ = file.flush();
                }
                
                // Keep running until signaled to stop
                while !stop_flag.load(Ordering::SeqCst) {
                    // Use timeout to allow for clean shutdown
                    match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(entry) => {
                            // Only log entries at or below our configured level
                            if entry.level <= file_level {
                                // Format the log line
                                let log_line = format!("{}\n", entry.format());
                                let line_size = log_line.len() as u64;
                                
                                // Check if we need to rotate logs
                                if current_size + line_size > max_file_size {
                                    // Flush before rotation to ensure no data loss
                                    if let Err(e) = file.flush() {
                                        eprintln!("Failed to flush log file before rotation: {}", e);
                                    }
                                
                                // Rotate logs
                                if use_backup {
                                    let backup_path = log_dir.join(format!("{}.log.bak", app_name));
                                    // Try to remove old backup first
                                    let _ = std::fs::remove_file(&backup_path);
                                    // Rename current log to backup
                                    if let Err(e) = std::fs::rename(&log_path, &backup_path) {
                                        eprintln!("Failed to rename log file: {}", e);
                                    }
                                } else {
                                    // Just delete the old log
                                    if let Err(e) = std::fs::remove_file(&log_path) {
                                        eprintln!("Failed to remove log file: {}", e);
                                    }
                                }
                                
                                // Create a new log file with retry mechanism
                                match open_log_file_with_retry(&log_path, 2) {
                                    Some(new_file) => {
                                        file = new_file;
                                        current_size = 0;
                                        
                                        // Write rotation marker
                                        let rotation_marker = format!("[{}] === Log rotated ===\n", 
                                            Local::now().format("%Y-%m-%d %H:%M:%S%.6f"));
                                        let _ = file.write_all(rotation_marker.as_bytes());
                                        let _ = file.flush();
                                    },
                                    None => {
                                        eprintln!("Failed to open new log file after rotation");
                                        
                                        // Try to recreate the log directory and retry with improved error handling
                                        if let Err(mkdir_err) = std::fs::create_dir_all(&log_dir) {
                                            eprintln!("Failed to create log directory: {}", mkdir_err);
                                        }
                                        
                                        // One more emergency attempt
                                        match OpenOptions::new()
                                            .create(true)
                                            .append(true)
                                            .open(&log_path) {
                                            Ok(retry_file) => {
                                                file = retry_file;
                                                current_size = 0;
                                                
                                                // Write recovery marker
                                                let recovery_marker = format!("[{}] === Log recovery after rotation ===\n", 
                                                    Local::now().format("%Y-%m-%d %H:%M:%S%.6f"));
                                                let _ = file.write_all(recovery_marker.as_bytes());
                                                let _ = file.flush();
                                            },
                                            Err(retry_err) => {
                                                eprintln!("Failed to open log file after retry (rotation): {}", retry_err);
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                            
                            // Write to file with improved reliability
                            let write_result = file.write_all(log_line.as_bytes());
                            if let Err(e) = write_result {
                                eprintln!("Failed to write to log file: {}", e);
                                
                                // Try to recover by reopening the file
                                match open_log_file_with_retry(&log_path, 1) {
                                    Some(recovery_file) => {
                                        file = recovery_file;
                                        // Try writing again with the recovered file
                                        if let Err(retry_e) = file.write_all(log_line.as_bytes()) {
                                            eprintln!("Failed to write to log file even after recovery: {}", retry_e);
                                        } else {
                                            // Successfully wrote on retry
                                            let _ = file.flush();
                                            current_size += line_size;
                                        }
                                    },
                                    None => {
                                        eprintln!("Failed to recover log file after write error");
                                    }
                                }
                            } else {
                                // Successfully wrote, now flush
                                if let Err(flush_e) = file.flush() {
                                    eprintln!("Failed to flush log file: {}", flush_e);
                                }
                                current_size += line_size;
                            }
                        }
                            
                            // Process any other pending entries
                            let mut batch_count = 0;
                            while let Ok(next_entry) = receiver.try_recv() {
                                if next_entry.level <= file_level {
                                    let next_line = format!("{}\n", next_entry.format());
                                    if let Err(e) = file.write_all(next_line.as_bytes()) {
                                        eprintln!("Failed to write batch log entry: {}", e);
                                        break;
                                    }
                                    batch_count += 1;
                                    if batch_count >= 10 {  // Process in reasonable batches
                                        // Flush after each batch
                                        let _ = file.flush();
                                        break;
                                    }
                                }
                            }
                            if batch_count > 0 {
                                let _ = file.flush();
                            }
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // Timeout is expected, just check if we should stop
                            continue;
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            // Channel disconnected, exit the worker
                            println!("File logger channel disconnected, exiting worker");
                            break;
                        }
                    }
                }
                
                // Process any remaining messages before shutting down
                println!("Processing remaining file logger messages before shutdown");
                while let Ok(entry) = receiver.try_recv() {
                    if entry.level <= file_level {
                        let log_line = format!("{}\n", entry.format());
                        if let Err(e) = file.write_all(log_line.as_bytes()) {
                            eprintln!("Failed to write final log entry: {}", e);
                        } else {
                            // Successfully wrote, update current size
                            current_size += log_line.len() as u64;
                        }
                    }
                }
                
                // Ensure log is finalized before shutdown
                let shutdown_message = format!("[{}] === Logger stopped ===\n", 
                    Local::now().format("%Y-%m-%d %H:%M:%S%.6f"));
                if let Err(e) = file.write_all(shutdown_message.as_bytes()) {
                    eprintln!("Failed to write shutdown message: {}", e);
                }
                if let Err(e) = file.flush() {
                    eprintln!("Failed to flush log file during final shutdown: {}", e);
                }
                
                // Ensure file is flushed before closing
                if let Err(e) = file.flush() {
                    eprintln!("Failed to flush log file during shutdown: {}", e);
                }
                
                println!("File log worker is shutting down");
            }) {
                Ok(handle) => handle,
                Err(e) => {
                    stop_flag_for_error.store(true, Ordering::SeqCst);
                    return Err(Error::thread_error(format!("Failed to start file worker: {}", e)));
                }
            };
        
        worker.handle = Some(handle);
        
        // Store the worker
        *self.file_worker.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock file worker: {}", e))
        )? = Some(worker);
        
        Ok(())
    }
    
    /// Set up the callback log worker
    fn setup_callback_worker(&self, receiver: Receiver<LogEntry>) -> Result<()> {
        // Get necessary values while holding the locks
        let callback_level = {
            let config = self.config.read().map_err(|e| 
                Error::lock_error(format!("Failed to lock logger config: {}", e))
            )?;
            
            match config.callback_level {
                Some(level) => level,
                None => return Ok(()), // Callback logging disabled
            }
        };
        
        // Get the callback function
        let callback = {
            let callback_guard = self.callback.lock().map_err(|e| 
                Error::lock_error(format!("Failed to lock callback: {}", e))
            )?;
            
            match callback_guard.clone() {
                Some(cb) => cb,
                None => return Ok(()), // No callback set
            }
        };
        
        let mut worker = LogWorker::new();
        let stop_flag = worker.stop_flag.clone();
        let stop_flag_for_error = worker.stop_flag.clone();
        
        let handle = match thread::Builder::new()
            .name("callback_logger".to_string())
            .spawn(move || {
                // Buffer no longer needed with direct processing
                
                // Keep running until signaled to stop
                while !stop_flag.load(Ordering::SeqCst) {
                    // Use timeout receive for the callback worker too for consistency
                    match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(entry) => {
                            if entry.level <= callback_level {
                                // Process the callback
                                let timestamp = entry.timestamp.format("%Y-%m-%d %H:%M:%S%.6f").to_string();
                                callback(entry.level, timestamp, entry.message.clone());
                                
                                // Process any batch of pending entries
                                let mut batch_count = 0;
                                while let Ok(next_entry) = receiver.try_recv() {
                                    if next_entry.level <= callback_level {
                                        let next_timestamp = next_entry.timestamp.format("%Y-%m-%d %H:%M:%S%.6f").to_string();
                                        callback(next_entry.level, next_timestamp, next_entry.message.clone());
                                        batch_count += 1;
                                        if batch_count >= 10 {  // Process up to 10 entries at once
                                            break;
                                        }
                                    }
                                }
                            }
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // Timeout is expected, just check if we should stop
                            continue;
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            // Channel disconnected, exit the worker
                            println!("Callback logger channel disconnected, exiting worker");
                            break;
                        }
                    }
                }
                
                // Process any remaining messages before shutting down
                println!("Processing remaining callback logger messages before shutdown");
                while let Ok(entry) = receiver.try_recv() {
                    if entry.level <= callback_level {
                        let timestamp = entry.timestamp.format("%Y-%m-%d %H:%M:%S%.6f").to_string();
                        callback(entry.level, timestamp, entry.message.clone());
                    }
                }
                
                // Clean up before terminating
                println!("Callback log worker is shutting down");
            }) {
                Ok(handle) => handle,
                Err(e) => {
                    stop_flag_for_error.store(true, Ordering::SeqCst);
                    return Err(Error::thread_error(format!("Failed to start callback worker: {}", e)));
                }
            };
        
        worker.handle = Some(handle);
        
        // Store the worker
        *self.callback_worker.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock callback worker: {}", e))
        )? = Some(worker);
        
        Ok(())
    }
    
    /// Log a message at the specified level
    pub fn log(&self, level: Level, message: impl Into<String>) -> Result<()> {
        if !self.is_running() {
            return Err(Error::logger_error("Logger is not running"));
        }
        
        let entry = LogEntry::new(message, level);
        self.log_entry(entry)
    }
    
    /// Log a pre-constructed entry
    pub fn log_entry(&self, entry: LogEntry) -> Result<()> {
        if !self.is_running() {
            return Err(Error::logger_error("Logger is not running"));
        }
        
        let sender_guard = self.sender.lock().map_err(|e| 
            Error::lock_error(format!("Failed to lock sender: {}", e))
        )?;
        
        let sender = sender_guard.as_ref()
            .ok_or_else(|| Error::logger_error("Logger sender is not initialized"))?;
        
        sender.send(entry).map_err(|_| 
            Error::logger_error("Failed to send log entry")
        )?;
        
        Ok(())
    }
}

/// Convenience functions for logging at different levels
impl Logger {
    /// Log an error message
    pub fn error<M: Into<String>>(&self, message: M) -> Result<()> {
        self.log(Level::Error, message)
    }
    
    /// Log a warning message
    pub fn warn<M: Into<String>>(&self, message: M) -> Result<()> {
        self.log(Level::Warn, message)
    }
    
    /// Log an info message
    pub fn info<M: Into<String>>(&self, message: M) -> Result<()> {
        self.log(Level::Info, message)
    }
    
    /// Log a debug message
    pub fn debug<M: Into<String>>(&self, message: M) -> Result<()> {
        self.log(Level::Debug, message)
    }
    
    /// Log a trace message
    pub fn trace<M: Into<String>>(&self, message: M) -> Result<()> {
        self.log(Level::Trace, message)
    }
}


/// Helper module with macros for logging
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => ({
        $crate::logger::Logger::instance().error(format!($($arg)*))
    })
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => ({
        $crate::logger::Logger::instance().warn(format!($($arg)*))
    })
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => ({
        $crate::logger::Logger::instance().info(format!($($arg)*))
    })
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => ({
        $crate::logger::Logger::instance().debug(format!($($arg)*))
    })
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => ({
        $crate::logger::Logger::instance().trace(format!($($arg)*))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    
    #[test]
    fn test_logger_config() {
        let config = LoggerConfig::default();
        assert_eq!(config.app_name, "application");
        assert!(config.console_level.is_some());
    }
    
    #[test]
    fn test_log_entry() {
        let entry = LogEntry::new("Test message", Level::Info);
        assert_eq!(entry.message, "Test message");
        assert_eq!(entry.level, Level::Info);
        
        let formatted = entry.format();
        assert!(formatted.contains("Test message"));
        assert!(formatted.contains("INFO"));
    }
    
    #[test]
    fn test_logger_start_stop() {
        // Make sure logger is stopped before test
        let logger = Logger::instance();
        logger.stop();
        
        // Sleep to ensure stop completes
        std::thread::sleep(Duration::from_millis(200));
        
        // Configure with minimal settings
        let config = LoggerConfig {
            app_name: "test_app".to_string(),
            console_level: None, // Disable console output for test
            file_level: None,    // Disable file output for test
            ..Default::default()
        };
        logger.configure(config);
        
        // Extra verification that we're starting from clean state
        // This could sometimes fail depending on timing between tests
        // since Logger is a singleton
        while logger.is_running() {
            std::thread::sleep(Duration::from_millis(100));
        }
        
        assert!(logger.start().is_ok());
        assert!(logger.is_running());
        
        logger.stop();
        
        // Give more time for the stop to complete (longer wait for stability)
        std::thread::sleep(Duration::from_millis(500));
        
        // Confirm it's stopped
        assert!(!logger.is_running());
    }
    
    #[test]
    fn test_logger_callback() {
        // Make sure logger is stopped before test
        let logger = Logger::instance();
        logger.stop();
        
        // Extra verification that we're starting from clean state
        std::thread::sleep(Duration::from_millis(100));
        assert!(!logger.is_running());
        
        // Configure with callback only
        let config = LoggerConfig {
            app_name: "test_app".to_string(),
            console_level: None, // Disable console output for test
            file_level: None,    // Disable file output for test
            callback_level: Some(Level::Info),
            ..Default::default()
        };
        logger.configure(config);
        
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        
        logger.set_callback(move |level, _, message| {
            if level == Level::Info && message == "Test callback message" {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            }
        });
        
        assert!(logger.start().is_ok());
        
        // Log a message
        assert!(logger.info("Test callback message").is_ok());
        
        // Give more time for the callback to be processed
        std::thread::sleep(Duration::from_millis(200));
        
        // Check the counter
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        
        // Stop the logger and wait for it to fully stop
        logger.stop();
        std::thread::sleep(Duration::from_millis(100));
    }
}