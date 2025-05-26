//! Demonstration of enhanced thread pool features.
//!
//! This example showcases the new capabilities added to rust_thread_system,
//! including job cancellation, unique job IDs, real-time metrics, and enhanced
//! priority system.

use rust_thread_system::{
    ThreadPool, PriorityThreadPool, JobPriority,
    job::{CallbackJob, ContextCallbackJob, JobId, Job},
    metrics::ThreadPoolMetrics,
};

use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
use std::time::Duration;
use std::thread;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Set up logger
    setup_logger()?;
    
    println!("🚀 Enhanced Rust Thread System Demo");
    println!("=====================================\n");
    
    // Demo 1: Job IDs and Context
    demo_job_ids_and_context()?;
    
    // Demo 2: Job Cancellation
    demo_job_cancellation()?;
    
    // Demo 3: Enhanced Priority System
    demo_enhanced_priorities()?;
    
    // Demo 4: Real-time Metrics
    demo_real_time_metrics()?;
    
    // Stop the logger
    rust_thread_system::logger::Logger::instance().stop();
    println!("\n✅ All demos completed successfully!");
    
    Ok(())
}

fn demo_job_ids_and_context() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("1️⃣  Job IDs and Context Demo");
    println!("---------------------------");
    
    let pool = ThreadPool::with_workers(2);
    pool.start()?;
    
    let counter = Arc::new(AtomicU32::new(0));
    let mut job_ids = Vec::new();
    
    // Submit jobs with context awareness
    for i in 0..5 {
        let counter_clone = counter.clone();
        
        let job = ContextCallbackJob::new(
            move |ctx| {
                println!("  📋 Job {} executing with ID: {}", i, ctx.id);
                counter_clone.fetch_add(1, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(100));
                println!("  ✅ Job {} completed (ID: {})", i, ctx.id);
                Ok(())
            },
            format!("context_job_{}", i)
        );
        
        job_ids.push(job.job_id().clone());
        pool.submit(job)?;
    }
    
    // Wait for completion
    thread::sleep(Duration::from_millis(600));
    pool.stop(true);
    
    println!("  📊 Total jobs completed: {}", counter.load(Ordering::Relaxed));
    println!("  🆔 Job IDs generated: {:?}", job_ids.len());
    println!();
    
    Ok(())
}

fn demo_job_cancellation() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("2️⃣  Job Cancellation Demo");
    println!("-----------------------");
    
    let pool = ThreadPool::with_workers(3);
    pool.start()?;
    
    let completed_counter = Arc::new(AtomicU32::new(0));
    let cancelled_counter = Arc::new(AtomicU32::new(0));
    
    let mut job_contexts = Vec::new();
    
    // Submit long-running jobs
    for i in 0..8 {
        let completed = completed_counter.clone();
        let cancelled = cancelled_counter.clone();
        
        let job = ContextCallbackJob::new(
            move |ctx| {
                println!("  🔄 Job {} started (ID: {})", i, ctx.id);
                
                // Simulate work with cancellation checks
                for step in 0..10 {
                    if ctx.is_cancelled() {
                        cancelled.fetch_add(1, Ordering::Relaxed);
                        println!("  ❌ Job {} cancelled at step {} (ID: {})", i, step, ctx.id);
                        return Err(rust_thread_system::error::Error::JobCancelled);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                
                completed.fetch_add(1, Ordering::Relaxed);
                println!("  ✅ Job {} completed (ID: {})", i, ctx.id);
                Ok(())
            },
            format!("cancellable_job_{}", i)
        );
        
        // Store job context for cancellation
        if let Some(ctx) = job.get_context() {
            job_contexts.push(ctx.clone());
        }
        
        pool.submit(job)?;
    }
    
    // Let some jobs start
    thread::sleep(Duration::from_millis(200));
    
    // Cancel half of the jobs
    println!("  🚫 Cancelling some jobs...");
    for ctx in job_contexts.iter().take(4) {
        ctx.cancel();
    }
    
    // Wait for remaining jobs to complete
    thread::sleep(Duration::from_millis(1000));
    pool.stop(true);
    
    println!("  📊 Jobs completed: {}", completed_counter.load(Ordering::Relaxed));
    println!("  📊 Jobs cancelled: {}", cancelled_counter.load(Ordering::Relaxed));
    println!();
    
    Ok(())
}

fn demo_enhanced_priorities() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("3️⃣  Enhanced Priority System Demo");
    println!("--------------------------------");
    
    let pool = PriorityThreadPool::with_workers(2);
    pool.start()?;
    
    let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));
    
    // Submit jobs in mixed priority order
    let priorities = [
        (JobPriority::Idle, "Idle"),
        (JobPriority::Critical, "Critical"),
        (JobPriority::Low, "Low"),
        (JobPriority::High, "High"),
        (JobPriority::Normal, "Normal"),
    ];
    
    for (priority, name) in priorities.iter() {
        let order = execution_order.clone();
        let priority_copy = *priority;
        let name_copy = name.to_string();
        
        let job = CallbackJob::new(
            move || {
                println!("  🎯 Executing {} priority job", name_copy);
                order.lock().unwrap().push(priority_copy);
                thread::sleep(Duration::from_millis(100));
                Ok(())
            },
            format!("{}_priority_job", name.to_lowercase())
        );
        
        let priority_job = rust_thread_system::priority_thread_pool::PriorityJob::new(
            job,
            *priority
        );
        
        pool.submit(priority_job)?;
        thread::sleep(Duration::from_millis(10)); // Small delay to ensure ordering
    }
    
    // Wait for all jobs to complete
    thread::sleep(Duration::from_millis(800));
    pool.stop(true);
    
    println!("  📊 Execution order (should be Critical→High→Normal→Low→Idle):");
    let order = execution_order.lock().unwrap();
    for (i, priority) in order.iter().enumerate() {
        println!("    {}. {:?}", i + 1, priority);
    }
    println!();
    
    Ok(())
}

#[cfg(feature = "metrics")]
fn demo_real_time_metrics() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("4️⃣  Real-time Metrics Demo");
    println!("------------------------");
    
    // Create metrics instance
    let metrics = Arc::new(ThreadPoolMetrics::new());
    let pool = PriorityThreadPool::with_workers(3);
    pool.start()?;
    
    let completed_jobs = Arc::new(AtomicU32::new(0));
    
    // Submit various priority jobs
    for i in 0..12 {
        let priority = match i % 3 {
            0 => JobPriority::High,
            1 => JobPriority::Normal,
            _ => JobPriority::Low,
        };
        
        let metrics_clone = metrics.clone();
        let completed = completed_jobs.clone();
        let _job_id = JobId::new();
        
        // Record job submission
        metrics.record_job_submitted(priority);
        
        let job = ContextCallbackJob::new(
            move |ctx| {
                // Record job start
                metrics_clone.record_job_started(ctx.id.clone(), priority);
                
                println!("  ⚡ Job {} executing with {} priority", i, priority);
                
                // Simulate varying work durations
                let work_time = match priority {
                    JobPriority::High => 50,
                    JobPriority::Normal => 100,
                    JobPriority::Low => 150,
                    _ => 100,
                };
                
                thread::sleep(Duration::from_millis(work_time));
                
                if fastrand::f32() < 0.1 {
                    // 10% chance of failure
                    metrics_clone.record_job_failed(&ctx.id);
                    return Err(rust_thread_system::error::Error::JobError("Random failure".to_string()));
                }
                
                // Record successful completion
                metrics_clone.record_job_completed(&ctx.id, priority);
                completed.fetch_add(1, Ordering::Relaxed);
                
                println!("  ✅ Job {} completed", i);
                Ok(())
            },
            format!("metrics_job_{}", i)
        );
        
        pool.submit(job)?;
        
        // Brief delay between submissions
        thread::sleep(Duration::from_millis(25));
    }
    
    // Monitor metrics in real-time
    for _ in 0..4 {
        thread::sleep(Duration::from_millis(500));
        
        let snapshot = metrics.snapshot();
        println!("  📊 Metrics update:");
        println!("    - Active jobs: {}", snapshot.active_jobs);
        println!("    - Completed: {}", snapshot.jobs_completed);
        println!("    - Failed: {}", snapshot.jobs_failed);
        println!("    - Success rate: {:.1}%", snapshot.success_rate * 100.0);
        println!("    - Total queue length: {}", 
                 snapshot.queue_lengths.values().sum::<usize>());
    }
    
    // Wait for all jobs to complete
    thread::sleep(Duration::from_millis(1000));
    pool.stop(true);
    
    // Final metrics summary
    println!("\n  📈 Final Metrics Summary:");
    let final_snapshot = metrics.snapshot();
    final_snapshot.print_summary();
    
    Ok(())
}

#[cfg(not(feature = "metrics"))]
fn demo_real_time_metrics() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("4️⃣  Real-time Metrics Demo");
    println!("------------------------");
    println!("  ⚠️  Metrics feature not enabled. Compile with --features metrics to see this demo.");
    println!();
    Ok(())
}

// Set up the logger (same as other examples)
fn setup_logger() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use rust_thread_system::logger::{Logger, LoggerConfig};
    use std::path::PathBuf;
    use log::Level;
    
    let config = LoggerConfig {
        app_name: "enhanced_features_demo".to_string(),
        max_file_size: 1024 * 1024, // 1 MB
        use_backup: true,
        check_interval: Duration::from_millis(50),
        log_dir: PathBuf::from("logs"),
        file_level: Some(Level::Info),
        console_level: Some(Level::Info),
        callback_level: None,
    };
    
    let logger = Logger::instance();
    logger.configure(config);
    logger.start()?;
    logger.init_global_logger().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    
    Ok(())
}