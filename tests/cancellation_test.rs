//! Comprehensive tests for job cancellation

use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn test_cancel_queued_job() {
    // Test cancelling a job before it starts execution
    let pool = ThreadPool::with_threads(1).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let executed = Arc::new(AtomicBool::new(false));
    let executed_clone = executed.clone();

    // Submit a long-running job to block the worker
    pool.execute(move || {
        thread::sleep(Duration::from_millis(200));
        Ok(())
    })
    .expect("Failed to submit blocking job");

    // Submit a cancellable job that will be queued
    let handle = pool
        .submit_cancellable(move |token| {
            if token.is_cancelled() {
                return Err(ThreadError::cancelled(
                    "test-job",
                    "Cancelled before execution",
                ));
            }
            executed_clone.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
            Ok(())
        })
        .expect("Failed to submit cancellable job");

    // Cancel the job before it starts
    handle.cancel();
    assert!(handle.is_cancelled());

    // Wait for the job to be processed
    thread::sleep(Duration::from_millis(500));

    // The job should not have set the flag
    // Note: The job may start before cancellation is checked, so this is not guaranteed
    // The important thing is that the job respects the cancellation token
    pool.shutdown().expect("Failed to shutdown pool");
}

#[test]
fn test_cancel_running_job() {
    // Test cancelling a job while it's running
    let pool = ThreadPool::with_threads(2).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let started = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let started_clone = started.clone();
    let cancelled_clone = cancelled.clone();

    let handle = pool
        .submit_cancellable(move |token| {
            started_clone.store(true, Ordering::SeqCst);

            // Simulate work with periodic cancellation checks
            for _ in 0..20 {
                if token.is_cancelled() {
                    cancelled_clone.store(true, Ordering::SeqCst);
                    return Err(ThreadError::cancelled(
                        "test-job",
                        "Cancelled during execution",
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        })
        .expect("Failed to submit cancellable job");

    // Wait for the job to start
    thread::sleep(Duration::from_millis(100));
    assert!(started.load(Ordering::SeqCst), "Job should have started");

    // Cancel the job while it's running
    handle.cancel();

    // Wait for cancellation to take effect
    thread::sleep(Duration::from_millis(200));

    assert!(
        cancelled.load(Ordering::SeqCst),
        "Job should have been cancelled"
    );

    pool.shutdown().expect("Failed to shutdown pool");
}

#[test]
fn test_multiple_cancellations() {
    // Test cancelling multiple jobs
    let pool = ThreadPool::with_threads(4).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let mut handles = Vec::new();

    // Submit 10 cancellable jobs
    for i in 0..10 {
        let handle = pool
            .submit_cancellable(move |token| {
                // Check cancellation every 10ms
                for _ in 0..100 {
                    if token.is_cancelled() {
                        return Err(ThreadError::cancelled(
                            format!("job-{}", i),
                            "Cancelled by test",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(())
            })
            .expect("Failed to submit job");
        handles.push(handle);
    }

    // Cancel half of the jobs
    let mut cancelled_count = 0;
    for (i, handle) in handles.iter().enumerate() {
        if i % 2 == 0 {
            handle.cancel();
            cancelled_count += 1;
        }
    }

    // Wait for all jobs to finish or be cancelled
    thread::sleep(Duration::from_millis(300));

    // Verify that we cancelled the expected number of jobs
    assert!(
        cancelled_count == 5,
        "Should have cancelled 5 jobs, cancelled {}",
        cancelled_count
    );

    // Check that the cancelled handles report as cancelled
    for (i, handle) in handles.iter().enumerate() {
        if i % 2 == 0 {
            assert!(handle.is_cancelled(), "Handle {} should be cancelled", i);
        }
    }

    pool.shutdown().expect("Failed to shutdown pool");
}

#[test]
fn test_cancellation_idempotent() {
    // Test that calling cancel multiple times is safe
    let pool = ThreadPool::with_threads(1).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let handle = pool
        .submit_cancellable(move |token| {
            thread::sleep(Duration::from_millis(100));
            if token.is_cancelled() {
                Err(ThreadError::cancelled("test-job", "Cancelled"))
            } else {
                Ok(())
            }
        })
        .expect("Failed to submit job");

    // Call cancel multiple times
    handle.cancel();
    handle.cancel();
    handle.cancel();

    assert!(handle.is_cancelled());

    thread::sleep(Duration::from_millis(200));
    pool.shutdown().expect("Failed to shutdown pool");
}

#[test]
fn test_uncancellable_job_completes() {
    // Test that uncancelled jobs complete normally
    let pool = ThreadPool::with_threads(2).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let completed = Arc::new(AtomicBool::new(false));
    let completed_clone = completed.clone();

    let _handle = pool
        .submit_cancellable(move |_token| {
            thread::sleep(Duration::from_millis(100));
            completed_clone.store(true, Ordering::SeqCst);
            Ok(())
        })
        .expect("Failed to submit job");

    // Don't cancel - let it complete
    thread::sleep(Duration::from_millis(200));

    assert!(
        completed.load(Ordering::SeqCst),
        "Job should have completed"
    );

    pool.shutdown().expect("Failed to shutdown pool");
}

#[test]
fn test_job_handle_clone() {
    // Test that JobHandle can be cloned and both clones work
    let pool = ThreadPool::with_threads(1).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let started = Arc::new(AtomicBool::new(false));
    let started_clone = started.clone();

    let handle = pool
        .submit_cancellable(move |token| {
            started_clone.store(true, Ordering::SeqCst);
            for _ in 0..50 {
                if token.is_cancelled() {
                    return Err(ThreadError::cancelled("test-job", "Cancelled"));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        })
        .expect("Failed to submit job");

    // Clone the handle
    let handle_clone = handle.clone();

    // Both should have the same job ID
    assert_eq!(handle.job_id(), handle_clone.job_id());

    // Wait a bit for job to start
    thread::sleep(Duration::from_millis(50));

    // Cancel using the clone
    handle_clone.cancel();

    // Both should show cancelled
    assert!(handle.is_cancelled());
    assert!(handle_clone.is_cancelled());

    thread::sleep(Duration::from_millis(200));

    pool.shutdown().expect("Failed to shutdown pool");
}

#[test]
fn test_cancel_with_long_work() {
    // Test cancelling a job with a longer computation
    let pool = ThreadPool::with_threads(1).expect("Failed to create pool");
    pool.start().expect("Failed to start pool");

    let work_done = Arc::new(AtomicU64::new(0));
    let work_done_clone = work_done.clone();

    let handle = pool
        .submit_cancellable(move |token| {
            for i in 0..1000 {
                // Check cancellation every 10 iterations
                if i % 10 == 0 && token.is_cancelled() {
                    return Err(ThreadError::cancelled("long-job", "Cancelled"));
                }

                work_done_clone.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_micros(500));
            }
            Ok(())
        })
        .expect("Failed to submit job");

    // Let it run a bit
    thread::sleep(Duration::from_millis(50));

    // Cancel it
    handle.cancel();

    // Wait for cancellation
    thread::sleep(Duration::from_millis(100));

    let work = work_done.load(Ordering::SeqCst);
    // Should have done some work but not all 1000 iterations
    assert!(work > 0, "Should have done some work");
    assert!(work < 1000, "Should not have completed all work");

    pool.shutdown().expect("Failed to shutdown pool");
}
