# Rust Thread System 성능 가이드

> **Languages**: [English](./PERFORMANCE.md) | 한국어

## 개요

이 가이드는 Rust Thread System의 성능 최적화에 대한 상세 정보를 제공합니다. 벤치마크, 튜닝 전략, best practice를 포함합니다.

## 목차

1. [성능 특성](#성능-특성)
2. [벤치마크](#벤치마크)
3. [워커 스레드 튜닝](#워커-스레드-튜닝)
4. [큐 설정](#큐-설정)
5. [작업 최적화](#작업-최적화)
6. [성능 모니터링](#성능-모니터링)
7. [일반적인 병목 현상](#일반적인-병목-현상)
8. [대안과의 비교](#대안과의-비교)

## 성능 특성

### 주요 메트릭

| 메트릭 | 일반적인 값 | 비고 |
|--------|---------------|-------|
| **작업 제출 지연시간** | < 10 μs | 큐에 작업 제출 시간 |
| **작업 실행 오버헤드** | < 50 μs | 작업당 스레드 풀 오버헤드 |
| **컨텍스트 스위치 시간** | 1-10 μs | OS 의존적 |
| **최대 처리량** | 1M+ 작업/초 | 최적 설정 시 |
| **워커당 메모리** | ~2 MB | 스택 + 메타데이터 |

### 확장성 특성

```
선형 확장 (1-8 워커):
- 처리량이 CPU 코어 수에 비례하여 증가
- 작업 큐에서 최소 경합

수익 감소 (8+ 워커):
- 큐 경합 증가
- 컨텍스트 스위칭 오버헤드
- 캐시 일관성 트래픽
```

## 벤치마크

### 테스트 환경

```
CPU: Intel Core i9-9900K (8코어, 16스레드)
RAM: 32 GB DDR4-3200
OS: Linux 5.15 (Ubuntu 22.04)
Rust: 1.75.0
```

### 처리량 벤치마크

#### 단순 작업 (No-op)

```rust
// 벤치마크: 최소 작업 실행
pool.execute(|| Ok(()))?;
```

| 워커 수 | 작업/초 | 지연시간 (μs) | CPU 사용량 |
|---------|----------|--------------|-----------|
| 1 | 250,000 | 4 | 100% (1코어) |
| 2 | 480,000 | 4 | 100% (2코어) |
| 4 | 920,000 | 4 | 100% (4코어) |
| 8 | 1,650,000 | 5 | 100% (8코어) |
| 16 | 1,850,000 | 9 | 80% (평균) |

**관찰:** 물리적 코어 수까지 거의 선형 확장.

#### CPU 집약적 작업

```rust
// 벤치마크: CPU 집약적 작업
pool.execute(|| {
    let mut sum = 0u64;
    for i in 0..10000 {
        sum = sum.wrapping_add(i);
    }
    Ok(())
})?;
```

| 워커 수 | 작업/초 | 작업/초/코어 | 효율성 |
|---------|----------|---------------|------------|
| 1 | 45,000 | 45,000 | 100% |
| 2 | 88,000 | 44,000 | 98% |
| 4 | 172,000 | 43,000 | 96% |
| 8 | 336,000 | 42,000 | 93% |

**관찰:** CPU 집약적 작업에서 우수한 확장성.

#### I/O 집약적 작업

```rust
// 벤치마크: 블로킹 I/O
pool.execute(|| {
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
})?;
```

| 워커 수 | 동시 작업 | 작업/초 | 비고 |
|---------|-----------------|----------|-------|
| 1 | 1 | 100 | 10ms당 1작업 |
| 4 | 4 | 400 | 4개 동시 |
| 16 | 16 | 1,600 | 16개 동시 |
| 64 | 64 | 6,400 | 높은 동시성 |

**관찰:** I/O 집약적 작업에서 워커 수에 비례하여 확장.

### 메모리 벤치마크

```rust
// 메모리 사용량 측정
let pool = ThreadPool::with_threads(8)?;
pool.start()?;

// 기본 메모리: ~16 MB (8 워커 × 2 MB 스택)
// 큐에 대기 중인 1000개 작업당: ~80 KB
// 추적된 완료 작업 1M개당: ~48 MB (통계)
```

### 지연시간 분포

```
작업 제출 (μs):
P50:  3.2 μs
P95:  8.1 μs
P99:  15.4 μs
P999: 45.2 μs

작업 실행 시작 (μs):
P50:  12.5 μs
P95:  89.3 μs
P99:  234.1 μs
P999: 1,203.5 μs
```

## 워커 스레드 튜닝

### 최적 워커 수 결정

#### CPU 집약적 워크로드

```rust
// 규칙: 물리적 CPU 코어당 1 워커
let num_workers = num_cpus::get_physical();

let pool = ThreadPool::with_threads(num_workers)?;
```

**근거:**
- 컨텍스트 스위칭 최소화
- CPU 활용도 최대화
- 캐시 스래싱 방지

#### I/O 집약적 워크로드

```rust
// 규칙: 워커 = 코어 × 블로킹 시간 비율
// 작업이 80% 시간 동안 블로킹되는 경우:
let num_workers = num_cpus::get() * 5;

let pool = ThreadPool::with_threads(num_workers)?;
```

**예제 계산:**

| 블로킹 시간 | 배수 | 8코어 시스템 |
|---------------|------------|---------------|
| 0% (순수 CPU) | 1× | 8 워커 |
| 50% | 2× | 16 워커 |
| 80% | 5× | 40 워커 |
| 90% | 10× | 80 워커 |
| 95% | 20× | 160 워커 |

#### 혼합 워크로드

```rust
// 물리적 코어로 시작, 메트릭 기반으로 조정
let num_workers = num_cpus::get_physical();

let config = ThreadPoolConfig::new(num_workers)
    .with_thread_name_prefix("mixed-worker");

let pool = ThreadPool::with_config(config)?;

// 모니터링 및 조정
let stats = pool.get_stats();
if stats.average_queue_size() > 100 {
    // 워커 추가 고려
}
```

### 워커 설정 Best Practices

```rust
// 프로덕션 설정
let config = ThreadPoolConfig::new(num_workers)
    .with_thread_name_prefix("worker")   // 디버깅용
    .with_stack_size(2 * 1024 * 1024)    // 2 MB 스택 (기본값)
    .with_max_queue_size(10000);          // 무제한 증가 방지

let pool = ThreadPool::with_config(config)?;
```

## 큐 설정

### 제한된 큐 vs 무제한 큐

#### 무제한 큐 (기본값)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(0); // 0 = 무제한

let pool = ThreadPool::with_config(config)?;
```

**장점:**
- 작업을 절대 거부하지 않음
- 사용이 간단
- 백프레셔 처리 불필요

**단점:**
- 메모리가 무제한으로 증가 가능
- 흐름 제어 없음
- 성능 문제를 숨길 수 있음

**사용 시기:**
- 작업 비율이 알려져 있고 제한됨
- 메모리가 풍부함
- 단순성이 선호됨

#### 제한된 큐

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(1000);

let pool = ThreadPool::with_config(config)?;
```

**장점:**
- 메모리 고갈 방지
- 백프레셔 처리 강제
- 과부하 조건 드러남

**단점:**
- 작업을 거부할 수 있음
- 에러 처리 필요
- 더 복잡함

**사용 시기:**
- 프로덕션 환경
- 알려지지 않거나 가변적인 부하
- 메모리 제한

### 큐 크기 튜닝

#### 작은 큐 (100-1000)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(100);
```

**특성:**
- 낮은 지연시간 (작업이 빨리 시작됨)
- 과부하에 대한 즉각적인 피드백
- 적극적인 백프레셔 처리 필요

**최적:**
- 지연시간에 민감한 애플리케이션
- 실시간 시스템
- 인터랙티브 애플리케이션

#### 중간 큐 (1,000-10,000)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(5000);
```

**특성:**
- 버스트 트래픽 처리
- 지연시간과 처리량의 균형
- 적당한 메모리 사용

**최적:**
- 웹 서버
- API 서비스
- 범용 애플리케이션

#### 큰 큐 (10,000+)

```rust
let config = ThreadPoolConfig::new(8)
    .with_max_queue_size(50000);
```

**특성:**
- 큰 버스트 처리
- 높은 지연시간 가능
- 더 많은 메모리 사용

**최적:**
- 배치 처리
- 백그라운드 작업 처리
- 높은 처리량 시스템

### 큐 크기 공식

```rust
// 공식: 큐 크기 = 워커 × 버스트 인수 × 평균 작업 시간 / 작업 간격
//
// 예제:
// - 8 워커
// - 5배 버스트 처리 원함
// - 평균 작업 시간 100ms
// - 정상 비율: 초당 10작업
// - 버스트 비율: 초당 50작업

let workers = 8;
let burst_factor = 5;
let avg_job_time_ms = 100;
let normal_rate = 10; // 작업/초

let queue_size = workers * burst_factor * (avg_job_time_ms / 1000) * normal_rate;
// = 8 × 5 × 0.1 × 10 = 40 작업

let config = ThreadPoolConfig::new(workers)
    .with_max_queue_size(queue_size);
```

## 작업 최적화

### 작업 오버헤드 최소화

#### 나쁨: 높은 오버헤드의 작은 작업

```rust
// 안티패턴: 너무 세분화됨
for i in 0..1000000 {
    pool.execute(move || {
        process_item(i); // 아주 작은 작업량
        Ok(())
    })?;
}
// 결과: 오버헤드가 실제 작업을 압도
```

#### 좋음: 작은 작업 배치 처리

```rust
// 패턴: 배치 처리
const BATCH_SIZE: usize = 1000;

for chunk in (0..1000000).collect::<Vec<_>>().chunks(BATCH_SIZE) {
    let chunk = chunk.to_vec();
    pool.execute(move || {
        for i in chunk {
            process_item(i);
        }
        Ok(())
    })?;
}
// 결과: 여러 항목에 걸쳐 오버헤드 분산
```

### 작업에서 블로킹 피하기

#### 나쁨: 블로킹 작업

```rust
pool.execute(|| {
    // 워커 스레드 블로킹!
    let response = reqwest::blocking::get("https://api.example.com")?;
    Ok(())
})?;
```

**영향:**
- I/O 중 워커 스레드 블로킹
- 효과적인 병렬성 감소
- 낮은 처리량

#### 좋음: 비동기 사용 또는 별도 I/O 풀

```rust
// 옵션 1: 별도 I/O 풀
let io_pool = ThreadPool::with_threads(100)?; // I/O용 많은 워커

io_pool.execute(|| {
    let response = reqwest::blocking::get("https://api.example.com")?;
    Ok(())
})?;

// 옵션 2: 비동기 런타임 사용 (예: Tokio)
// CPU 풀과 I/O를 분리 유지
```

### 효율적인 에러 처리

```rust
// 효율적: 최소 할당
pool.execute(|| {
    if some_condition {
        return Err(ThreadError::execution("실패"));
    }
    Ok(())
})?;

// 피하기: 비용이 많이 드는 에러 컨텍스트
pool.execute(|| {
    if some_condition {
        // 핫 패스에서 하지 말 것
        let context = format!("{:?}", expensive_debug_info);
        return Err(ThreadError::execution(context));
    }
    Ok(())
})?;
```

## 성능 모니터링

### 내장 통계 사용

```rust
let pool = ThreadPool::with_threads(8)?;
pool.start()?;

// 작업 제출...

// 풀 레벨 통계
let total_submitted = pool.total_jobs_submitted();
let total_processed = pool.total_jobs_processed();
let total_failed = pool.total_jobs_failed();

println!("처리량: {} 작업/초",
    total_processed as f64 / elapsed_time.as_secs_f64());

// 워커별 통계
let worker_stats = pool.get_stats();
for (i, stat) in worker_stats.iter().enumerate() {
    println!("워커 {}:", i);
    println!("  처리됨: {}", stat.get_jobs_processed());
    println!("  실패: {}", stat.get_jobs_failed());
    println!("  평균 시간: {:.2} μs", stat.get_average_processing_time_us());
}
```

### 주요 성능 지표

#### 처리량

```rust
let start = Instant::now();
let jobs_before = pool.total_jobs_processed();

// ... 일정 시간 실행 ...

let jobs_after = pool.total_jobs_processed();
let elapsed = start.elapsed();

let throughput = (jobs_after - jobs_before) as f64 / elapsed.as_secs_f64();
println!("처리량: {:.0} 작업/초", throughput);
```

#### 큐 깊이

```rust
// 시간에 따른 큐 크기 모니터링
let queue_size = pool.queued_jobs();

if queue_size > 1000 {
    println!("경고: 큐가 쌓이고 있음 ({})", queue_size);
    // 고려사항: 워커 추가, 작업 비율 감소, 또는 작업 최적화
}
```

#### 워커 활용도

```rust
let stats = pool.get_stats();
let active_workers = stats.iter()
    .filter(|s| s.is_active())
    .count();

let utilization = (active_workers as f64 / stats.len() as f64) * 100.0;
println!("워커 활용도: {:.1}%", utilization);

// 지속적으로 < 50%: 워커 감소 고려
// 지속적으로 > 95%: 워커 추가 고려
```

#### 성공률

```rust
let total_processed = pool.total_jobs_processed();
let total_failed = pool.total_jobs_failed();
let total = total_processed + total_failed;

let success_rate = if total > 0 {
    (total_processed as f64 / total as f64) * 100.0
} else {
    0.0
};

println!("성공률: {:.2}%", success_rate);
```

## 일반적인 병목 현상

### 1. 큐 경합

**증상:** 높은 CPU 사용량이지만 낮은 처리량

**진단:**
```rust
// 제출이 느린지 확인
let start = Instant::now();
pool.execute(|| Ok(()))?;
let submit_time = start.elapsed();

if submit_time > Duration::from_micros(100) {
    println!("큐 경합 감지됨!");
}
```

**해결책:**
- 워커 수 감소
- work-stealing 큐 사용 (향후 개선사항)
- 작업 제출 배치

### 2. 워커 기아

**증상:** 큐에 작업이 있는데 일부 워커가 유휴 상태

**진단:**
```rust
let stats = pool.get_stats();
let idle_workers = stats.iter()
    .filter(|s| !s.is_active() && pool.queued_jobs() > 0)
    .count();

if idle_workers > 0 {
    println!("워커 기아: {} 유휴", idle_workers);
}
```

**해결책:**
- 데드락 확인
- 작업이 무기한 블로킹하지 않도록 보장
- 큐 크기 증가

### 3. 메모리 압박

**증상:** 높은 메모리 사용, OOM 가능

**진단:**
```rust
let queued = pool.queued_jobs();
let memory_estimate_mb = queued * 1024 / 1024 / 1024; // 대략적인 추정

if memory_estimate_mb > 100 {
    println!("경고: 높은 메모리 사용 ({} MB)", memory_estimate_mb);
}
```

**해결책:**
- 제한된 큐 사용
- 백프레셔 구현
- 작업 제출 비율 감소

### 4. Thundering Herd

**증상:** 주기적인 활동 급증 후 유휴

**해결책:** 작업 제출 분산
```rust
// 나쁨: 한 번에 모두 제출
for i in 0..10000 {
    pool.execute(|| Ok(()))?;
}

// 좋음: 제출 속도 제한
use std::time::{Duration, Instant};

let mut last_submit = Instant::now();
let min_interval = Duration::from_micros(10);

for i in 0..10000 {
    while last_submit.elapsed() < min_interval {
        std::thread::yield_now();
    }

    pool.execute(|| Ok(()))?;
    last_submit = Instant::now();
}
```

## 대안과의 비교

### vs. Rayon

| 기능 | rust_thread_system | rayon |
|---------|-------------------|-------|
| **제어** | 세밀한 제어 | 거친 제어 |
| **작업 추적** | 예 (통계) | 아니오 |
| **에러 처리** | 작업별 Result | Panic 전파 |
| **동적 제어** | 예 | 제한적 |
| **사용 사례** | 장기 실행 서비스 | 데이터 병렬성 |

### vs. Tokio

| 기능 | rust_thread_system | tokio |
|---------|-------------------|-------|
| **모델** | 스레드 풀 | 비동기 런타임 |
| **블로킹** | OK | 권장 안 함 |
| **CPU 집약적** | 우수 | 양호 |
| **I/O 집약적** | 양호 | 우수 |
| **복잡도** | 낮음 | 높음 |

### 성능 비교

```
벤치마크: 1M개 단순 CPU 집약적 작업

rust_thread_system (8 워커): 1.2초
rayon (8 스레드):              0.9초
tokio (8 워커):              1.8초

rust_thread_system: 혼합 워크로드에 적합
rayon: 순수 데이터 병렬성에 최적
tokio: I/O 중심 비동기 워크로드에 최적
```

## Best Practices 요약

1. **워커 풀 적정 크기 설정**
   - CPU 집약적: 물리적 코어 수
   - I/O 집약적: 코어 × 블로킹 비율

2. **프로덕션에서 제한된 큐 사용**
   - 메모리 문제 방지
   - 적절한 백프레셔 강제

3. **작은 작업 배치 처리**
   - 오버헤드 감소
   - 처리량 향상

4. **성능 모니터링**
   - 처리량, 큐 깊이, 활용도 추적
   - 이상 징후에 대한 경고 설정

5. **블로킹 작업 피하기**
   - I/O에 별도 풀 사용
   - 또는 비동기 런타임 사용

6. **부하 테스트**
   - 현실적인 워크로드로 벤치마크
   - 프로덕션 전에 한계점 찾기

---

*성능 가이드 버전 1.0*
*최종 업데이트: 2025-10-16*
