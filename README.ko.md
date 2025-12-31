# Rust Thread System

> **Languages**: [English](./README.md) | 한국어

워커 풀과 작업 큐를 갖춘 프로덕션 준비 완료된 고성능 Rust 스레딩 프레임워크입니다.

## 주요 기능

- **Thread Pool 관리**: 설정 가능한 스레드 수를 가진 효율적인 워커 풀
- **유연한 Job Queue**: 제한된/무제한/적응형 작업 큐 지원
- **Backpressure 전략**: 큐 가득 참 상황에 대한 설정 가능한 처리 (차단, 타임아웃, 거부, 삭제)
- **계층적 취소 토큰**: 부모-자식 토큰 관계, 연쇄 취소, 타임아웃 및 콜백 지원
- **워커 통계**: 워커별 및 풀 전체의 포괄적인 메트릭 추적
- **고성능**: 최적의 성능을 위해 crossbeam 채널과 parking_lot 기반으로 구축
- **스레드 안전성**: 가능한 경우 Lock-free 방식, 최소한의 동기화 오버헤드
- **우아한 종료**: 워커 스레드의 적절한 join을 통한 깨끗한 종료
- **타입 안전 Job**: 컴파일 타임 안전성을 갖춘 Trait 기반 작업 시스템
- **커스텀 Job**: 완전한 제어를 갖춘 커스텀 작업 타입을 쉽게 구현

## 빠른 시작

`Cargo.toml`에 추가:

```toml
[dependencies]
rust_thread_system = "0.1.0"
```

기본 사용법:

```rust
use rust_thread_system::prelude::*;

fn main() -> Result<()> {
    // Create and start a thread pool
    let pool = ThreadPool::with_threads(4)?;
    pool.start()?;

    // Submit jobs using closures
    for i in 0..10 {
        pool.execute(move || {
            println!("Job {} executing", i);
            Ok(())
        })?;
    }

    // Graceful shutdown
    pool.shutdown()?;
    Ok(())
}
```

## 아키텍처

### 핵심 구성 요소

- **Job Trait**: 실행될 작업 단위 정의
- **ThreadPool**: 워커 스레드와 작업 분배 관리
- **Worker**: 작업을 처리하는 개별 워커 스레드
- **WorkerStats**: 워커별 통계 및 메트릭

### 설계 원칙

1. **Zero-cost 추상화**: 작업 제출 및 실행에 대한 최소한의 오버헤드
2. **타입 안전성**: 작업 처리에 대한 컴파일 타임 보장
3. **우아한 성능 저하**: 풀을 중단시키지 않고 오류 처리
4. **관찰 가능성**: 모니터링 및 디버깅을 위한 풍부한 통계

## 사용 예제

### 기본 Thread Pool

```rust
use rust_thread_system::prelude::*;

let pool = ThreadPool::with_threads(4)?;
pool.start()?;

pool.execute(|| {
    println!("Hello from worker thread!");
    Ok(())
})?;

pool.shutdown()?;
```

### 커스텀 설정

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(1000)
    .with_thread_name_prefix("my-worker")
    .with_poll_interval(Duration::from_millis(50));  // 더 빠른 응답성

let pool = ThreadPool::with_config(config)?;
pool.start()?;
```

#### Poll Interval 튜닝

`poll_interval`은 워커가 새 작업과 종료 신호를 확인하는 빈도를 제어합니다:

- **고처리량 시스템** (10-50ms): 빠른 작업 수신, 높은 CPU 사용량
- **백그라운드 서비스** (500ms-1s): 낮은 CPU 사용량, 느린 종료
- **기본값** (100ms): 대부분의 사용 사례에 균형 잡힌 설정

### 커스텀 Job 타입

```rust
use rust_thread_system::prelude::*;

struct DataProcessingJob {
    data: Vec<u32>,
}

impl Job for DataProcessingJob {
    fn execute(&mut self) -> Result<()> {
        let sum: u32 = self.data.iter().sum();
        println!("Sum: {}", sum);
        Ok(())
    }

    fn job_type(&self) -> &str {
        "DataProcessingJob"
    }
}

// Submit custom job
pool.submit(DataProcessingJob {
    data: vec![1, 2, 3, 4, 5],
})?;
```

### 제한된 큐

```rust
use rust_thread_system::prelude::*;

// Create pool with bounded queue to prevent memory exhaustion
let config = ThreadPoolConfig::new(4).with_max_queue_size(100);
let pool = ThreadPool::with_config(config)?;
pool.start()?;

// Jobs will be rejected if queue is full
match pool.execute(|| Ok(())) {
    Ok(()) => println!("Job accepted"),
    Err(ThreadError::ShuttingDown { .. }) => println!("Queue full or shutting down"),
    Err(e) => println!("Error: {}", e),
}
```

### Non-Blocking 작업 제출

```rust
use rust_thread_system::prelude::*;
use std::time::Duration;

let config = ThreadPoolConfig::new(4).with_max_queue_size(100);
let pool = ThreadPool::with_config(config)?;
pool.start()?;

// try_execute는 큐가 가득 차면 즉시 반환
match pool.try_execute(|| {
    println!("Job executed");
    Ok(())
}) {
    Ok(()) => println!("Job submitted"),
    Err(ThreadError::QueueFull { current, max }) => {
        println!("큐가 가득 찼습니다 ({}/{}), 나중에 다시 시도하세요", current, max);
    },
    Err(e) => println!("Error: {}", e),
}

// execute_timeout은 지정된 시간 동안 큐 공간을 기다림
match pool.execute_timeout(|| {
    println!("Job executed");
    Ok(())
}, Duration::from_millis(100)) {
    Ok(()) => println!("Job submitted"),
    Err(ThreadError::SubmissionTimeout { timeout_ms }) => {
        println!("제출 시간 초과: {}ms", timeout_ms);
    },
    Err(e) => println!("Error: {}", e),
}

pool.shutdown()?;
```

### 워커 통계

```rust
use rust_thread_system::prelude::*;

// Get per-worker statistics
let stats = pool.get_stats();
for (i, stat) in stats.iter().enumerate() {
    println!("Worker {}: {} jobs processed, {} failed",
        i,
        stat.get_jobs_processed(),
        stat.get_jobs_failed()
    );
    println!("  Average processing time: {:.2}μs",
        stat.get_average_processing_time_us()
    );
}

// Get pool-wide statistics
println!("Total jobs submitted: {}", pool.total_jobs_submitted());
println!("Total jobs processed: {}", pool.total_jobs_processed());
println!("Total jobs failed: {}", pool.total_jobs_failed());
```

## 예제

`examples/` 디렉토리에는 여러 완전한 예제가 포함되어 있습니다:

- **basic_usage.rs**: 클로저를 사용한 간단한 thread pool 사용법
- **custom_jobs.rs**: 커스텀 작업 타입 구현
- **bounded_queue.rs**: 메모리 사용량을 제한하기 위한 제한된 큐 사용

예제 실행:

```bash
cargo run --example basic_usage
cargo run --example custom_jobs
cargo run --example bounded_queue
```

## 성능 특성

- **작업 제출**: O(1) 분할 상환
- **워커 스케줄링**: 무제한 큐 사용 시 Lock-free
- **메모리 오버헤드**: 최소 - 작업당이 아닌 풀당 하나의 채널
- **종료 지연 시간**: 가장 긴 실행 작업에 의해 제한됨

### 벤치마크

벤치마크 실행:

```bash
cargo bench
```

예상 성능 (최신 하드웨어 기준):
- 작업 제출: 작업당 약 1-2μs
- 작업 실행 오버헤드: <1μs
- 처리량: 초당 수백만 개의 작업 (작업 복잡도에 따라 다름)

## 스레드 안전성

모든 public API는 스레드 안전합니다:

- `ThreadPool`은 다중 프로듀서 시나리오를 위해 `Arc`를 통해 공유 가능
- 작업 제출은 무제한 큐에 대해 Lock-free
- 워커 통계는 최소한의 오버헤드를 위해 atomic 연산 사용

## 오류 처리

라이브러리는 포괄적인 오류 타입을 사용합니다:

```rust
pub enum ThreadError {
    AlreadyRunning { pool_name, worker_count },
    NotRunning { pool_name },
    ShuttingDown { pending_jobs },
    SpawnError { thread_id, message, source },
    JoinError { thread_id, message },
    ExecutionError { job_id, message },
    Cancelled { job_id, reason },
    JobTimeout { job_id, timeout_ms },
    QueueFull { current, max },
    QueueSendError,
    SubmissionTimeout { timeout_ms },
    InvalidConfig { parameter, message },
    WorkerPanic { thread_id, message },
    PoolExhausted { active, total },
    Other(String),
}
```

모든 오류는 `thiserror`를 통해 `std::error::Error`를 구현합니다.

## 대안과의 비교

| 기능 | rust_thread_system | rayon | threadpool |
|---------|-------------------|-------|------------|
| 커스텀 작업 타입 | ✅ | ❌ | ❌ |
| 워커 통계 | ✅ | ❌ | ❌ |
| 제한된 큐 | ✅ | N/A | ❌ |
| 우아한 종료 | ✅ | ✅ | ⚠️ |
| 데이터 병렬화 | ❌ | ✅ | ❌ |

## 의존성

- **crossbeam**: 고성능 동시성 채널
- **parking_lot**: 더 빠른 동기화 프리미티브
- **thiserror**: 인체공학적인 오류 처리
- **num_cpus**: CPU 개수 감지

## 라이선스

이 프로젝트는 BSD 3-Clause 라이선스에 따라 라이선스가 부여됩니다. 자세한 내용은 LICENSE 파일을 참조하세요.

## 기여

기여를 환영합니다! 이슈나 Pull Request를 자유롭게 제출해 주세요.

## 저자

Thread System Team

## 참고

- [C++ thread_system](https://github.com/kcenon/thread_system) - 원본 C++ 구현
- [rust_container_system](../rust_container_system) - 동반 Rust 컨테이너 라이브러리
- [rust_database_system](../rust_database_system) - 동반 Rust 데이터베이스 라이브러리
- [rust_logger_system](../rust_logger_system) - 동반 Rust 로거 라이브러리
