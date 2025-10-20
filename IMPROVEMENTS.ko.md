# Rust Thread System - 개선 계획

> **Languages**: [English](./IMPROVEMENTS.md) | 한국어

## 개요

이 문서는 코드 분석을 바탕으로 식별된 Rust Thread System의 약점과 제안된 개선사항을 설명합니다.

## 식별된 문제점

### 1. 무제한 큐 증가 위험

**문제**: 기본 무제한 큐는 작업이 워커가 처리할 수 있는 속도보다 빠르게 제출되면 무한정 증가하여, 메모리 고갈과 시스템 불안정을 초래할 수 있습니다.

**위치**: `src/pool.rs:45`

**현재 구현**:
```rust
pub fn new(num_threads: usize) -> Self {
    let (sender, receiver) = crossbeam_channel::unbounded();  // 무제한!
    // ...
}
```

**영향**:
- 높은 부하에서 메모리가 제한 없이 증가
- 작업 제출을 늦추는 역압(backpressure) 메커니즘 없음
- 프로덕션에서 메모리 부족 크래시 가능성
- 메모리 누수와 정상적인 큐 증가 구별 어려움

**제안된 해결책**:

**옵션 1: 설정 가능한 크기로 제한된 큐를 기본으로**

```rust
// TODO: 무제한 큐를 합리적인 제한된 기본값으로 교체
// 역압과 함께 큐 용량 설정 추가

pub struct ThreadPoolConfig {
    pub num_threads: usize,
    pub queue_capacity: Option<usize>,  // None은 무제한 (opt-in)
    pub backpressure_strategy: BackpressureStrategy,
}

pub enum BackpressureStrategy {
    Block,           // 큐가 가득 차면 발신자 차단
    DropOldest,      // 공간을 만들기 위해 가장 오래된 작업 삭제
    DropNewest,      // 들어오는 작업 삭제
    ReturnError,     // 호출자에게 에러 반환
}

impl ThreadPool {
    pub fn new(num_threads: usize) -> Self {
        // 합리적인 용량을 가진 제한된 큐를 기본으로
        Self::with_config(ThreadPoolConfig {
            num_threads,
            queue_capacity: Some(num_threads * 100),  // 스레드당 100개 작업
            backpressure_strategy: BackpressureStrategy::Block,
        })
    }

    pub fn with_config(config: ThreadPoolConfig) -> Self {
        let (sender, receiver) = match config.queue_capacity {
            Some(capacity) => crossbeam_channel::bounded(capacity),
            None => crossbeam_channel::unbounded(),  // 명시적 opt-in
        };

        // ... 나머지 초기화

        Self {
            workers: vec![],
            sender,
            receiver,
            config,
        }
    }

    pub fn submit(&self, job: Box<dyn Job + Send>) -> Result<(), SubmitError> {
        match self.config.backpressure_strategy {
            BackpressureStrategy::Block => {
                self.sender.send(job).map_err(|_| SubmitError::PoolShutdown)?;
                Ok(())
            }
            BackpressureStrategy::ReturnError => {
                self.sender
                    .try_send(job)
                    .map_err(|e| match e {
                        TrySendError::Full(_) => SubmitError::QueueFull,
                        TrySendError::Disconnected(_) => SubmitError::PoolShutdown,
                    })
            }
            // ... 다른 전략들
        }
    }
}

#[derive(Debug)]
pub enum SubmitError {
    QueueFull,
    PoolShutdown,
}
```

**옵션 2: 모니터링 및 알림 추가**

```rust
// TODO: 큐 깊이 모니터링 및 알림 추가

pub struct ThreadPool {
    // ... 기존 필드들
    queue_depth: Arc<AtomicUsize>,
    max_queue_depth: Arc<AtomicUsize>,
    queue_full_events: Arc<AtomicU64>,
}

impl ThreadPool {
    pub fn submit(&self, job: Box<dyn Job + Send>) -> Result<(), SubmitError> {
        let current_depth = self.queue_depth.fetch_add(1, Ordering::Relaxed);

        // 관찰된 최대 큐 깊이 업데이트
        self.max_queue_depth.fetch_max(current_depth + 1, Ordering::Relaxed);

        // 큐가 위험할 정도로 커지면 알림
        if current_depth > 10000 {
            log::warn!(
                "Thread pool queue depth critical: {} jobs pending",
                current_depth
            );
        }

        self.sender.send(job).map_err(|_| SubmitError::PoolShutdown)?;
        Ok(())
    }

    pub fn queue_depth(&self) -> usize {
        self.queue_depth.load(Ordering::Relaxed)
    }

    pub fn max_queue_depth(&self) -> usize {
        self.max_queue_depth.load(Ordering::Relaxed)
    }

    pub fn queue_statistics(&self) -> QueueStatistics {
        QueueStatistics {
            current_depth: self.queue_depth(),
            max_depth: self.max_queue_depth(),
            full_events: self.queue_full_events.load(Ordering::Relaxed),
        }
    }
}
```

**우선순위**: 높음
**예상 작업량**: 중간 (1주)

### 2. 스레드 네이밍 또는 식별 없음

**문제**: 워커 스레드에 이름이 지정되지 않아 디버깅, 프로파일링, 모니터링이 어렵습니다. 스레드 덤프와 프로파일링 도구가 의미 있는 식별자 대신 일반적인 스레드 이름을 표시합니다.

**위치**: `src/pool.rs:78`

**현재 구현**:
```rust
for id in 0..num_threads {
    let receiver = Arc::clone(&receiver);
    let handle = thread::spawn(move || {  // 익명 스레드!
        // ... 워커 루프
    });
    workers.push(Worker { id, handle });
}
```

**영향**:
- 프로파일러(perf, Instruments 등)에서 스레드 식별 어려움
- 스레드 덤프가 "thread '<unnamed>'"와 같은 도움이 안 되는 이름 표시
- 스레드 활동을 애플리케이션 동작과 쉽게 연관시킬 수 없음
- 데드락 및 성능 문제 디버깅 어려움

**제안된 해결책**:

```rust
// TODO: 디버깅 및 프로파일링을 위한 의미 있는 스레드 이름 추가

for id in 0..num_threads {
    let receiver = Arc::clone(&receiver);

    let thread_name = format!("thread-pool-worker-{}", id);

    let handle = thread::Builder::new()
        .name(thread_name.clone())  // 이름 있는 스레드!
        .spawn(move || {
            log::debug!("Worker thread {} started", thread_name);

            loop {
                match receiver.recv() {
                    Ok(job) => {
                        log::trace!("Worker {} executing job", thread_name);
                        job.execute();
                    }
                    Err(_) => {
                        log::debug!("Worker {} shutting down", thread_name);
                        break;
                    }
                }
            }
        })
        .expect("Failed to spawn worker thread");

    workers.push(Worker { id, handle });
}
```

**사용자 정의 풀 이름을 가진 향상된 버전**:

```rust
pub struct ThreadPoolConfig {
    pub num_threads: usize,
    pub pool_name: Option<String>,  // 사용자 정의 풀 식별자
    // ... 다른 설정
}

impl ThreadPool {
    pub fn with_config(config: ThreadPoolConfig) -> Self {
        let pool_name = config.pool_name
            .unwrap_or_else(|| format!("pool-{}", POOL_COUNTER.fetch_add(1, Ordering::Relaxed)));

        for id in 0..config.num_threads {
            let thread_name = format!("{}-worker-{}", pool_name, id);

            let handle = thread::Builder::new()
                .name(thread_name)
                .spawn(move || {
                    // ... 워커 루프
                })
                .expect("Failed to spawn worker thread");

            workers.push(Worker { id, handle });
        }

        // ...
    }
}

// 사용법:
let pool = ThreadPool::with_config(ThreadPoolConfig {
    num_threads: 4,
    pool_name: Some("http-handlers".to_string()),
    // ... 다른 설정
});

// 다음 이름으로 스레드 생성:
// - http-handlers-worker-0
// - http-handlers-worker-1
// - http-handlers-worker-2
// - http-handlers-worker-3
```

**우선순위**: 중간
**예상 작업량**: 소 (1-2일)

### 3. 제한된 에러 처리

**문제**: 작업 패닉은 잡히지만 로그만 남기고, 에러 복구, 재시도 또는 호출자에게 알림을 보내는 메커니즘이 없습니다.

**현재 구현**:
```rust
match receiver.recv() {
    Ok(job) => {
        // 패닉 복구나 에러 처리 없음
        job.execute();
    }
    Err(_) => break,
}
```

**영향**:
- 패닉된 작업이 로그 출력만으로 조용히 실패
- 실패한 작업을 재시도할 방법 없음
- 작업 실패를 호출자에게 알릴 수 없음
- 견고한 에러 처리 패턴 구현 어려움

**제안된 해결책**:

```rust
// TODO: 포괄적인 에러 처리 및 복구 추가

pub trait Job: Send {
    fn execute(&self) -> Result<(), JobError>;

    fn on_error(&self, error: &JobError) -> ErrorRecovery {
        ErrorRecovery::Log  // 기본 동작
    }
}

#[derive(Debug)]
pub enum JobError {
    Panic(String),
    ExecutionError(Box<dyn std::error::Error + Send>),
}

pub enum ErrorRecovery {
    Log,                          // 에러만 로그
    Retry { max_attempts: usize }, // 백오프와 함께 재시도
    Callback(Box<dyn Fn(JobError) + Send>), // 사용자 정의 콜백
}

impl ThreadPool {
    pub fn submit_with_callback<F>(
        &self,
        job: Box<dyn Job + Send>,
        on_complete: F,
    ) -> Result<(), SubmitError>
    where
        F: Fn(Result<(), JobError>) + Send + 'static,
    {
        let wrapped_job = CallbackJob {
            inner: job,
            callback: Box::new(on_complete),
        };

        self.sender
            .send(Box::new(wrapped_job))
            .map_err(|_| SubmitError::PoolShutdown)
    }
}

struct CallbackJob<F>
where
    F: Fn(Result<(), JobError>) + Send,
{
    inner: Box<dyn Job + Send>,
    callback: Box<F>,
}

impl<F> Job for CallbackJob<F>
where
    F: Fn(Result<(), JobError>) + Send,
{
    fn execute(&self) -> Result<(), JobError> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.inner.execute()
        }));

        let job_result = match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(panic) => {
                let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                Err(JobError::Panic(msg))
            }
        };

        (self.callback)(job_result.clone());
        job_result
    }
}

// 사용법:
pool.submit_with_callback(
    Box::new(MyJob::new()),
    |result| {
        match result {
            Ok(()) => println!("Job completed successfully"),
            Err(e) => eprintln!("Job failed: {:?}", e),
        }
    },
)?;
```

**우선순위**: 중간
**예상 작업량**: 중간 (3-5일)

## 추가 개선사항

자세한 내용은 [영문 버전](./IMPROVEMENTS.md)의 추가 개선사항 섹션을 참조하세요:

- 스레드 풀 통계 및 모니터링
- 동적 스레드 풀 크기 조정
- 작업 우선순위

## 테스트 요구사항

### 필요한 새 테스트:

1. **큐 역압 테스트**:
   ```rust
   #[test]
   fn test_bounded_queue_blocks() {
       let pool = ThreadPool::with_config(ThreadPoolConfig {
           num_threads: 2,
           queue_capacity: Some(10),
           backpressure_strategy: BackpressureStrategy::Block,
       });

       // 큐를 채움
       for i in 0..10 {
           pool.submit(Box::new(SlowJob::new(Duration::from_secs(10)))).unwrap();
       }

       // 다음 제출은 작업이 완료될 때까지 차단되어야 함
       let start = Instant::now();
       pool.submit(Box::new(QuickJob::new())).unwrap();
       let elapsed = start.elapsed();

       // 잠시 차단되었어야 함
       assert!(elapsed > Duration::from_millis(100));
   }
   ```

2. **스레드 네이밍 테스트**:
   ```rust
   #[test]
   fn test_thread_names() {
       let pool = ThreadPool::with_config(ThreadPoolConfig {
           num_threads: 4,
           pool_name: Some("test-pool".to_string()),
           ..Default::default()
       });

       // 스레드 이름을 확인하는 작업 제출
       let (tx, rx) = std::sync::mpsc::channel();

       pool.submit(Box::new(move || {
           let thread_name = std::thread::current().name().unwrap().to_string();
           tx.send(thread_name).unwrap();
       })).unwrap();

       let thread_name = rx.recv().unwrap();
       assert!(thread_name.starts_with("test-pool-worker-"));
   }
   ```

3. **에러 처리 테스트**:
   ```rust
   #[test]
   fn test_panic_recovery() {
       let pool = ThreadPool::new(2);

       let (tx, rx) = std::sync::mpsc::channel();
       let tx_clone = tx.clone();

       // 패닉하는 작업 제출
       pool.submit_with_callback(
           Box::new(|| panic!("intentional panic")),
           move |result| {
               tx_clone.send(result).unwrap();
           },
       ).unwrap();

       // 에러를 받아야 함
       let result = rx.recv().unwrap();
       assert!(matches!(result, Err(JobError::Panic(_))));

       // 풀은 여전히 작동해야 함
       pool.submit(Box::new(|| {
           tx.send(Ok(())).unwrap();
       })).unwrap();

       let result = rx.recv().unwrap();
       assert!(result.is_ok());
   }
   ```

## 구현 로드맵

### 1단계: 중요 안전성 (스프린트 1)
- [ ] 무제한 큐를 제한된 기본값으로 교체
- [ ] 역압 전략 추가
- [ ] 큐 깊이 모니터링 추가
- [ ] 포괄적인 큐 테스트 추가

### 2단계: 관찰성 (스프린트 2)
- [ ] 스레드 네이밍 구현
- [ ] 통계 추적 추가
- [ ] 상태 확인 API 생성
- [ ] 모니터링 문서 추가

### 3단계: 에러 처리 (스프린트 3)
- [ ] 패닉 복구 구현
- [ ] 에러 콜백 추가
- [ ] 작업 재시도 로직 지원
- [ ] 에러 처리 테스트 추가

### 4단계: 고급 기능 (스프린트 4-5)
- [ ] 동적 크기 조정 구현
- [ ] 작업 우선순위 추가
- [ ] 성능 벤치마크 생성
- [ ] 고급 사용 예제 추가

## Breaking Changes

⚠️ **주의**: 제한된 큐를 기본으로 변경하는 것은 호환성을 깨는 변경입니다.

**마이그레이션 경로**:
1. 버전 1.x: 무제한을 기본으로 유지, deprecated 경고 추가
2. 버전 1.x: `ThreadPool::bounded()` 및 `ThreadPool::unbounded()` 생성자 추가
3. 버전 2.0: 제한된 큐를 기본으로, 무제한에 대한 명시적 opt-in 필요
4. 예제와 함께 CHANGELOG에 마이그레이션 문서화

## 성능 목표

### 현재 성능:
- 작업 제출 지연: ~1μs (무제한 큐)
- 작업 실행 오버헤드: ~5μs
- 컨텍스트 스위치 오버헤드: 작업당 ~10μs

### 개선 후 목표 성능:
- 작업 제출 지연: ~2μs (제한된 큐, 허용 가능한 오버헤드)
- 작업 실행 오버헤드: ~5μs (변경 없음)
- 메모리 사용: 제한되고 예측 가능
- 큐 깊이 모니터링: <1% 오버헤드

## 참고자료

- 코드 분석: Thread System Review 2025-10-16
- 관련 이슈:
  - 무제한 큐 위험 (#TODO)
  - 스레드 네이밍 (#TODO)
  - 에러 처리 (#TODO)
- crossbeam-channel 문서: https://docs.rs/crossbeam-channel/

---

*개선 계획 버전 1.0*
*최종 업데이트: 2025-10-17*
