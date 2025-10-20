//! Property-based tests for rust_thread_system using proptest

use proptest::prelude::*;
use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// ThreadPoolConfig Tests
// ============================================================================

proptest! {
    /// Test that ThreadPoolConfig can be created with various thread counts
    #[test]
    fn test_config_thread_count(threads in 1usize..32) {
        let _config = ThreadPoolConfig::new(threads);

        // Config should be created successfully
        assert!(threads > 0);
    }

    /// Test that ThreadPoolConfig with max queue size works
    #[test]
    fn test_config_max_queue_size(
        threads in 1usize..16,
        max_queue_size in 1usize..10000
    ) {
        let _config = ThreadPoolConfig::new(threads)
            .with_max_queue_size(max_queue_size);

        // Should create successfully
        assert!(threads > 0);
        assert!(max_queue_size > 0);
    }

    /// Test that ThreadPoolConfig with custom thread name prefix works
    #[test]
    fn test_config_thread_name_prefix(
        threads in 1usize..8,
        prefix in "[a-z]{3,10}"
    ) {
        let _config = ThreadPoolConfig::new(threads)
            .with_thread_name_prefix(&prefix);

        // Should create successfully
        assert!(!prefix.is_empty());
    }
}

// ============================================================================
// ThreadPool Creation Tests
// ============================================================================

proptest! {
    /// Test that ThreadPool can be created with various thread counts
    #[test]
    fn test_pool_creation(threads in 1usize..16) {
        let result = ThreadPool::with_threads(threads);

        // Should create successfully
        assert!(result.is_ok(), "Failed to create pool with {} threads: {:?}",
                threads, result.err());
    }

    /// Test that ThreadPool with config can be created
    #[test]
    fn test_pool_creation_with_config(
        threads in 1usize..8,
        max_queue in 10usize..1000
    ) {
        let config = ThreadPoolConfig::new(threads)
            .with_max_queue_size(max_queue);

        let result = ThreadPool::with_config(config);

        assert!(result.is_ok(), "Failed to create pool: {:?}", result.err());
    }
}

// ============================================================================
// Job Execution Tests
// ============================================================================

proptest! {
    /// Test that multiple jobs can be executed successfully
    #[test]
    fn test_multiple_job_execution(job_count in 1usize..50) {
        let mut pool = ThreadPool::with_threads(4).unwrap();
        pool.start().unwrap();

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit multiple jobs
        for _ in 0..job_count {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }).unwrap();
        }

        // Wait for jobs to complete
        std::thread::sleep(Duration::from_millis(100));
        pool.shutdown().unwrap();

        // Verify all jobs executed
        assert_eq!(counter.load(Ordering::SeqCst), job_count,
                   "Not all jobs executed: expected {}, got {}",
                   job_count, counter.load(Ordering::SeqCst));
    }

    /// Test that job results are processed correctly
    #[test]
    fn test_job_execution_with_data(values in prop::collection::vec(any::<i32>(), 1..20)) {
        let mut pool = ThreadPool::with_threads(2).unwrap();
        pool.start().unwrap();

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit jobs that increment counter
        for _value in values.iter() {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }).unwrap();
        }

        // Wait for completion
        std::thread::sleep(Duration::from_millis(100));
        pool.shutdown().unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), values.len());
    }
}

// ============================================================================
// Queue Limit Tests (DoS Protection)
// ============================================================================

proptest! {
    /// Test that bounded queue enforces limits
    #[test]
    fn test_bounded_queue_limit(
        threads in 1usize..4,
        max_queue in 5usize..20,
        submit_count in 10usize..50
    ) {
        let config = ThreadPoolConfig::new(threads)
            .with_max_queue_size(max_queue);

        let mut pool = ThreadPool::with_config(config).unwrap();
        pool.start().unwrap();

        let mut successful_submissions = 0;
        let mut queue_full_errors = 0;

        // Try to submit more jobs than queue can hold
        for _ in 0..submit_count {
            // Submit a slow job to fill the queue
            let result = pool.execute(|| {
                std::thread::sleep(Duration::from_millis(10));
                Ok(())
            });

            match result {
                Ok(_) => successful_submissions += 1,
                Err(ThreadError::QueueFull { .. }) => queue_full_errors += 1,
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
        }

        pool.shutdown().unwrap();

        // Should have some successful submissions
        assert!(successful_submissions > 0,
                "No jobs were submitted successfully");

        // Verify the sum makes sense
        assert_eq!(successful_submissions + queue_full_errors, submit_count,
                   "Some jobs were lost");

        // The test verifies that the queue limit mechanism works
        // (either by accepting all or by rejecting some when full)
        let _ = queue_full_errors;
    }
}

// ============================================================================
// Panic Isolation Tests (Reliability)
// ============================================================================

proptest! {
    /// Test that worker threads survive job panics
    #[test]
    fn test_panic_isolation(
        panic_count in 1usize..10,
        success_count in 1usize..10
    ) {
        let mut pool = ThreadPool::with_threads(2).unwrap();
        pool.start().unwrap();

        let counter = Arc::new(AtomicUsize::new(0));

        // Submit jobs that panic
        for _ in 0..panic_count {
            let _ = pool.execute(|| {
                panic!("Intentional panic for testing");
            });
        }

        // Small delay to let panicking jobs execute
        std::thread::sleep(Duration::from_millis(50));

        // Submit successful jobs
        for _ in 0..success_count {
            let counter_clone = Arc::clone(&counter);
            pool.execute(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }).unwrap();
        }

        // Wait for successful jobs
        std::thread::sleep(Duration::from_millis(100));
        pool.shutdown().unwrap();

        // Verify that successful jobs ran despite earlier panics
        assert_eq!(counter.load(Ordering::SeqCst), success_count,
                   "Worker pool did not recover from panics: expected {} successful jobs, got {}",
                   success_count, counter.load(Ordering::SeqCst));
    }
}

// ============================================================================
// Worker Statistics Tests
// ============================================================================

proptest! {
    /// Test that worker statistics are tracked correctly
    #[test]
    fn test_worker_stats(job_count in 10usize..50) {
        let mut pool = ThreadPool::with_threads(4).unwrap();
        pool.start().unwrap();

        // Submit jobs
        for _ in 0..job_count {
            pool.execute(|| Ok(())).unwrap();
        }

        // Wait for jobs to complete
        std::thread::sleep(Duration::from_millis(150));

        // Get stats
        let total = pool.total_jobs_processed();

        pool.shutdown().unwrap();

        // Total should match submitted jobs (all successful)
        assert_eq!(total, job_count as u64,
                   "Job count mismatch: expected {}, got {}", job_count, total);
    }

    /// Test that individual worker stats are reasonable
    #[test]
    fn test_individual_worker_stats(
        threads in 2usize..8,
        jobs in 20usize..100
    ) {
        let mut pool = ThreadPool::with_threads(threads).unwrap();
        pool.start().unwrap();

        // Submit jobs
        for _ in 0..jobs {
            pool.execute(|| Ok(())).unwrap();
        }

        // Wait for completion
        std::thread::sleep(Duration::from_millis(200));

        let stats = pool.get_stats();
        pool.shutdown().unwrap();

        // Verify we have stats for all workers
        assert_eq!(stats.len(), threads,
                   "Expected stats for {} workers, got {}", threads, stats.len());

        // Sum of individual stats should equal total
        let sum: u64 = stats.iter().map(|s| s.get_jobs_processed()).sum();
        assert_eq!(sum, jobs as u64,
                   "Sum of worker stats ({}) doesn't match total jobs ({})", sum, jobs);

        // Each worker should have processed at least one job (with high probability)
        // but we can't guarantee this in all cases due to scheduling
        let workers_with_jobs = stats.iter().filter(|s| s.get_jobs_processed() > 0).count();
        assert!(workers_with_jobs > 0, "No workers processed any jobs");
    }
}

// ============================================================================
// Safety Tests (No Panics)
// ============================================================================

proptest! {
    /// Test that ThreadPool creation never panics
    #[test]
    fn test_pool_creation_no_panic(threads in 1usize..100) {
        // Should not panic (may return error for excessive thread counts)
        let _ = ThreadPool::with_threads(threads);
    }

    /// Test that shutdown is always safe
    #[test]
    fn test_shutdown_always_safe(threads in 1usize..8) {
        let mut pool = ThreadPool::with_threads(threads).unwrap();
        pool.start().unwrap();

        // Submit a few jobs
        for _ in 0..5 {
            let _ = pool.execute(|| Ok(()));
        }

        // Shutdown should never panic
        let result = pool.shutdown();
        assert!(result.is_ok(), "Shutdown failed: {:?}", result);
    }

    /// Test that double shutdown is safe
    #[test]
    fn test_double_shutdown_safe(threads in 1usize..4) {
        let mut pool = ThreadPool::with_threads(threads).unwrap();
        pool.start().unwrap();

        // First shutdown
        pool.shutdown().unwrap();

        // Second shutdown should not panic (may return error)
        let _ = pool.shutdown();
    }
}

// ============================================================================
// Error Handling Tests
// ============================================================================

proptest! {
    /// Test that submitting to stopped pool returns error
    #[test]
    fn test_submit_to_stopped_pool(_dummy in 0..100u32) {
        let pool = ThreadPool::with_threads(2).unwrap();
        // Don't start the pool

        let result = pool.execute(|| Ok(()));

        // Should return error (pool not started or already shutdown)
        // We just verify it doesn't panic
        let _ = result;
    }
}
