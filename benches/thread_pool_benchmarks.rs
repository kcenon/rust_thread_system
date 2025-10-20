use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use rust_thread_system::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn benchmark_thread_pool_creation(c: &mut Criterion) {
    c.bench_function("thread_pool_creation", |b| {
        b.iter(|| {
            let mut pool = ThreadPool::with_threads(4).expect("Failed to create pool");
            pool.start().expect("Failed to start pool");
            pool.shutdown().expect("Failed to shutdown pool");
        });
    });
}

fn benchmark_job_submission(c: &mut Criterion) {
    let mut group = c.benchmark_group("job_submission");

    // Lightweight jobs
    group.bench_function("lightweight_jobs_100", |b| {
        b.iter_batched(
            || {
                let mut pool = ThreadPool::with_threads(4).expect("Failed to create pool");
                pool.start().expect("Failed to start pool");
                pool
            },
            |mut pool| {
                for _ in 0..100 {
                    pool.execute(|| {
                        black_box(1 + 1);
                        Ok(())
                    })
                    .expect("Failed to submit job");
                }
                pool.shutdown().expect("Failed to shutdown pool");
            },
            BatchSize::SmallInput,
        );
    });

    // Medium workload
    group.bench_function("medium_jobs_100", |b| {
        b.iter_batched(
            || {
                let mut pool = ThreadPool::with_threads(4).expect("Failed to create pool");
                pool.start().expect("Failed to start pool");
                pool
            },
            |mut pool| {
                for _ in 0..100 {
                    pool.execute(|| {
                        // Simulate some work
                        let mut sum = 0u64;
                        for i in 0..1000 {
                            sum = sum.wrapping_add(i);
                        }
                        black_box(sum);
                        Ok(())
                    })
                    .expect("Failed to submit job");
                }
                pool.shutdown().expect("Failed to shutdown pool");
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn benchmark_concurrent_submission(c: &mut Criterion) {
    c.bench_function("concurrent_submission_4_threads", |b| {
        b.iter_batched(
            || {
                let mut pool = ThreadPool::with_threads(4).expect("Failed to create pool");
                pool.start().expect("Failed to start pool");
                Arc::new(pool)
            },
            |pool| {
                let handles: Vec<_> = (0..4)
                    .map(|_| {
                        let pool = Arc::clone(&pool);
                        std::thread::spawn(move || {
                            for _ in 0..25 {
                                pool.execute(|| Ok(())).expect("Failed to submit job");
                            }
                        })
                    })
                    .collect();

                for handle in handles {
                    handle.join().expect("Thread panicked");
                }

                // Need to get mutable access for shutdown
                let mut pool = Arc::try_unwrap(pool).expect("Failed to unwrap Arc");
                pool.shutdown().expect("Failed to shutdown pool");
            },
            BatchSize::SmallInput,
        );
    });
}

fn benchmark_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("tasks_per_second", |b| {
        b.iter_batched(
            || {
                let mut pool = ThreadPool::with_threads(8).expect("Failed to create pool");
                pool.start().expect("Failed to start pool");
                let counter = Arc::new(AtomicU64::new(0));
                (pool, counter)
            },
            |(mut pool, counter)| {
                // Submit 1000 tasks
                for _ in 0..1000 {
                    let counter = Arc::clone(&counter);
                    pool.execute(move || {
                        counter.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    })
                    .expect("Failed to submit job");
                }

                pool.shutdown().expect("Failed to shutdown pool");

                // Verify all tasks completed
                let total = counter.load(Ordering::Relaxed);
                assert_eq!(total, 1000, "Not all tasks completed");
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn benchmark_bounded_queue(c: &mut Criterion) {
    c.bench_function("bounded_queue_pressure", |b| {
        b.iter_batched(
            || {
                let config = ThreadPoolConfig::new(4).with_max_queue_size(100);
                let mut pool = ThreadPool::with_config(config).expect("Failed to create pool");
                pool.start().expect("Failed to start pool");
                pool
            },
            |mut pool| {
                // Try to submit more than queue size
                for _ in 0..150 {
                    let _ = pool.execute(|| {
                        std::thread::sleep(Duration::from_micros(100));
                        Ok(())
                    });
                }
                pool.shutdown().expect("Failed to shutdown pool");
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    benchmark_thread_pool_creation,
    benchmark_job_submission,
    benchmark_concurrent_submission,
    benchmark_throughput,
    benchmark_bounded_queue
);
criterion_main!(benches);
