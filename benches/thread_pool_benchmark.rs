use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use rust_thread_system::{ThreadPool, job::CallbackJob};
use std::time::Duration;

fn thread_pool_job_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("thread_pool");
    
    for worker_count in [1, 2, 4, 8].iter() {
        group.bench_with_input(BenchmarkId::new("job_throughput", worker_count), worker_count, |b, &workers| {
            let pool = ThreadPool::with_workers(workers);
            pool.start().expect("Failed to start thread pool");
            
            b.iter(|| {
                // Submit 100 simple jobs
                for i in 0..100 {
                    let job = CallbackJob::new(move || {
                        // Simulate a very small amount of work
                        std::thread::sleep(Duration::from_micros(10));
                        Ok(())
                    }, format!("bench_job_{}", i));
                    
                    pool.submit(job).expect("Failed to submit job");
                }
                
                // Allow time for jobs to complete
                std::thread::sleep(Duration::from_millis(20));
            });
            
            pool.stop(true);
        });
    }
    
    group.finish();
}

criterion_group!(benches, thread_pool_job_benchmark);
criterion_main!(benches);