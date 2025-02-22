//! Backoff strategies for job retries.
//!
//! This module provides a set of backoff strategies that can be used to implement
//! retry patterns for jobs that fail to execute successfully.

use std::fmt;
use std::time::{Duration, Instant};

/// Trait defining the interface for backoff strategies.
///
/// A backoff strategy determines how long to wait before retrying a failed operation.
pub trait BackoffStrategy: Send + Sync + fmt::Debug {
    /// Calculate the next backoff duration based on the current attempt number.
    ///
    /// # Arguments
    ///
    /// * `attempt` - The current retry attempt number, starting from 1 for the first retry.
    ///
    /// # Returns
    ///
    /// The duration to wait before the next retry attempt, or `None` if no more retries should be performed.
    fn next_backoff(&self, attempt: u32) -> Option<Duration>;

    /// Reset the internal state of the backoff strategy, if any.
    fn reset(&mut self) {}

    /// Create a boxed clone of self for use in dynamic contexts.
    fn clone_box(&self) -> Box<dyn BackoffStrategy>;

    /// Get the maximum number of retry attempts this strategy allows.
    fn max_attempts(&self) -> u32;
}

impl Clone for Box<dyn BackoffStrategy> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// A backoff strategy that always waits the same amount of time between retries.
#[derive(Debug, Clone)]
pub struct ConstantBackoff {
    /// The constant duration to wait between retries.
    duration: Duration,
    /// The maximum number of retry attempts.
    max_retry_count: u32,
}

impl ConstantBackoff {
    /// Create a new constant backoff strategy with the specified duration.
    ///
    /// # Arguments
    ///
    /// * `duration` - The duration to wait between retry attempts.
    /// * `max_retry_count` - The maximum number of retry attempts.
    pub fn new(duration: Duration, max_retry_count: u32) -> Self {
        Self {
            duration,
            max_retry_count,
        }
    }
}

impl BackoffStrategy for ConstantBackoff {
    fn next_backoff(&self, attempt: u32) -> Option<Duration> {
        if attempt <= self.max_retry_count {
            Some(self.duration)
        } else {
            None
        }
    }

    fn clone_box(&self) -> Box<dyn BackoffStrategy> {
        Box::new(self.clone())
    }

    fn max_attempts(&self) -> u32 {
        self.max_retry_count
    }
}

/// A backoff strategy that increases the waiting time linearly.
#[derive(Debug, Clone)]
pub struct LinearBackoff {
    /// The initial duration to wait.
    initial_duration: Duration,
    /// The increment to add to the duration for each retry.
    increment: Duration,
    /// The maximum number of retry attempts.
    max_retry_count: u32,
}

impl LinearBackoff {
    /// Create a new linear backoff strategy.
    ///
    /// # Arguments
    ///
    /// * `initial_duration` - The duration to wait for the first retry.
    /// * `increment` - The increment to add to the duration for each retry.
    /// * `max_retry_count` - The maximum number of retry attempts.
    pub fn new(initial_duration: Duration, increment: Duration, max_retry_count: u32) -> Self {
        Self {
            initial_duration,
            increment,
            max_retry_count,
        }
    }
}

impl BackoffStrategy for LinearBackoff {
    fn next_backoff(&self, attempt: u32) -> Option<Duration> {
        if attempt <= self.max_retry_count {
            let multiplier = attempt - 1;
            let duration = self.initial_duration + self.increment * multiplier;
            Some(duration)
        } else {
            None
        }
    }

    fn clone_box(&self) -> Box<dyn BackoffStrategy> {
        Box::new(self.clone())
    }

    fn max_attempts(&self) -> u32 {
        self.max_retry_count
    }
}

/// A backoff strategy that increases the waiting time exponentially.
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    /// The initial duration to wait.
    initial_duration: Duration,
    /// The multiplier to apply to the duration for each retry.
    multiplier: f32,
    /// The maximum duration to wait.
    max_duration: Duration,
    /// The maximum number of retry attempts.
    max_retry_count: u32,
    /// Optional random jitter factor (0.0 to 1.0) to add randomness to backoff times.
    jitter: Option<f32>,
}

impl ExponentialBackoff {
    /// Create a new exponential backoff strategy.
    ///
    /// # Arguments
    ///
    /// * `initial_duration` - The duration to wait for the first retry.
    /// * `multiplier` - The factor by which to multiply the duration for each retry.
    /// * `max_duration` - The maximum duration to wait for any retry.
    /// * `max_retry_count` - The maximum number of retry attempts.
    /// * `jitter` - Optional factor (0.0 to 1.0) to add randomness to backoff times.
    pub fn new(
        initial_duration: Duration,
        multiplier: f32,
        max_duration: Duration,
        max_retry_count: u32,
        jitter: Option<f32>,
    ) -> Self {
        Self {
            initial_duration,
            multiplier,
            max_duration,
            max_retry_count,
            jitter,
        }
    }
}

impl BackoffStrategy for ExponentialBackoff {
    fn next_backoff(&self, attempt: u32) -> Option<Duration> {
        if attempt <= self.max_retry_count {
            let exponent = attempt - 1;
            let multiplier = self.multiplier.powi(exponent as i32);
            
            let base_millis = self.initial_duration.as_millis() as f32 * multiplier;
            let capped_millis = base_millis.min(self.max_duration.as_millis() as f32);
            
            let duration = if let Some(jitter) = self.jitter {
                // Add random jitter, between (1-jitter) and (1+jitter) of the original value
                let rand_factor = 1.0 - jitter + fastrand::f32() * jitter * 2.0;
                Duration::from_millis((capped_millis * rand_factor) as u64)
            } else {
                Duration::from_millis(capped_millis as u64)
            };
            
            Some(duration)
        } else {
            None
        }
    }

    fn clone_box(&self) -> Box<dyn BackoffStrategy> {
        Box::new(self.clone())
    }

    fn max_attempts(&self) -> u32 {
        self.max_retry_count
    }
}

/// A composite backoff strategy that applies different strategies for different retry phases.
#[derive(Debug, Clone)]
pub struct CompositeBackoff {
    /// The strategies to apply in sequence.
    strategies: Vec<Box<dyn BackoffStrategy>>,
    /// The cumulative number of retries for each strategy.
    cumulative_attempts: Vec<u32>,
}

impl CompositeBackoff {
    /// Create a new composite backoff strategy with the given sequence of strategies.
    pub fn new(strategies: Vec<Box<dyn BackoffStrategy>>) -> Self {
        let mut cumulative_attempts = Vec::with_capacity(strategies.len());
        let mut sum = 0;
        
        for strategy in &strategies {
            sum += strategy.max_attempts();
            cumulative_attempts.push(sum);
        }
        
        Self {
            strategies,
            cumulative_attempts,
        }
    }
}

impl BackoffStrategy for CompositeBackoff {
    fn next_backoff(&self, attempt: u32) -> Option<Duration> {
        if self.strategies.is_empty() || attempt == 0 {
            return None;
        }

        let mut prev_cum = 0;
        
        for (i, &cum) in self.cumulative_attempts.iter().enumerate() {
            if attempt <= cum {
                // This is the relevant strategy
                let adjusted_attempt = attempt - prev_cum;
                return self.strategies[i].next_backoff(adjusted_attempt);
            }
            prev_cum = cum;
        }
        
        None
    }

    fn reset(&mut self) {
        for strategy in &mut self.strategies {
            strategy.reset();
        }
    }

    fn clone_box(&self) -> Box<dyn BackoffStrategy> {
        Box::new(self.clone())
    }

    fn max_attempts(&self) -> u32 {
        self.cumulative_attempts.last().copied().unwrap_or(0)
    }
}

/// A retry policy that determines how many times to retry a job and with what backoff strategy.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// The backoff strategy to use for retries.
    strategy: Box<dyn BackoffStrategy>,
    /// The current retry attempt number.
    current_attempt: u32,
    /// The time at which the next retry should be performed.
    next_retry_time: Option<Instant>,
}

impl RetryPolicy {
    /// Create a new retry policy with the given backoff strategy.
    pub fn new(strategy: Box<dyn BackoffStrategy>) -> Self {
        Self {
            strategy,
            current_attempt: 0,
            next_retry_time: None,
        }
    }
    
    /// Create a retry policy with a constant backoff strategy.
    pub fn constant(duration: Duration, max_retries: u32) -> Self {
        Self::new(Box::new(ConstantBackoff::new(duration, max_retries)))
    }
    
    /// Create a retry policy with a linear backoff strategy.
    pub fn linear(initial: Duration, increment: Duration, max_retries: u32) -> Self {
        Self::new(Box::new(LinearBackoff::new(initial, increment, max_retries)))
    }
    
    /// Create a retry policy with an exponential backoff strategy.
    pub fn exponential(
        initial: Duration,
        multiplier: f32,
        max_duration: Duration,
        max_retries: u32,
        jitter: Option<f32>,
    ) -> Self {
        Self::new(Box::new(ExponentialBackoff::new(
            initial,
            multiplier,
            max_duration,
            max_retries,
            jitter,
        )))
    }
    
    /// Record a failed attempt and calculate the next retry time.
    ///
    /// Returns true if a retry should be attempted, false if max retries reached.
    pub fn record_failure(&mut self) -> bool {
        self.current_attempt += 1;
        
        if let Some(backoff) = self.strategy.next_backoff(self.current_attempt) {
            self.next_retry_time = Some(Instant::now() + backoff);
            true
        } else {
            self.next_retry_time = None;
            false
        }
    }
    
    /// Check if it's time to retry the job.
    pub fn is_ready(&self) -> bool {
        match self.next_retry_time {
            Some(time) => Instant::now() >= time,
            None => false,
        }
    }
    
    /// Get the current retry attempt number.
    pub fn current_attempt(&self) -> u32 {
        self.current_attempt
    }
    
    /// Get the maximum number of retry attempts.
    pub fn max_attempts(&self) -> u32 {
        self.strategy.max_attempts()
    }
    
    /// Reset the retry policy to its initial state.
    pub fn reset(&mut self) {
        self.current_attempt = 0;
        self.next_retry_time = None;
        self.strategy.reset();
    }
    
    /// Get the time remaining until the next retry is ready.
    pub fn time_remaining(&self) -> Option<Duration> {
        self.next_retry_time.map(|time| {
            let now = Instant::now();
            if time > now {
                time - now
            } else {
                Duration::from_secs(0)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_constant_backoff() {
        let strategy = ConstantBackoff::new(Duration::from_millis(100), 3);
        
        assert_eq!(strategy.next_backoff(1), Some(Duration::from_millis(100)));
        assert_eq!(strategy.next_backoff(2), Some(Duration::from_millis(100)));
        assert_eq!(strategy.next_backoff(3), Some(Duration::from_millis(100)));
        assert_eq!(strategy.next_backoff(4), None);
    }
    
    #[test]
    fn test_linear_backoff() {
        let strategy = LinearBackoff::new(
            Duration::from_millis(100),
            Duration::from_millis(50),
            3,
        );
        
        assert_eq!(strategy.next_backoff(1), Some(Duration::from_millis(100)));
        assert_eq!(strategy.next_backoff(2), Some(Duration::from_millis(150)));
        assert_eq!(strategy.next_backoff(3), Some(Duration::from_millis(200)));
        assert_eq!(strategy.next_backoff(4), None);
    }
    
    #[test]
    fn test_exponential_backoff() {
        let strategy = ExponentialBackoff::new(
            Duration::from_millis(100),
            2.0,
            Duration::from_millis(1000),
            3,
            None,
        );
        
        assert_eq!(strategy.next_backoff(1), Some(Duration::from_millis(100)));
        assert_eq!(strategy.next_backoff(2), Some(Duration::from_millis(200)));
        assert_eq!(strategy.next_backoff(3), Some(Duration::from_millis(400)));
        assert_eq!(strategy.next_backoff(4), None);
    }
    
    #[test]
    fn test_composite_backoff() {
        let strategies: Vec<Box<dyn BackoffStrategy>> = vec![
            Box::new(ConstantBackoff::new(Duration::from_millis(100), 2)),
            Box::new(LinearBackoff::new(
                Duration::from_millis(200),
                Duration::from_millis(50),
                2,
            )),
        ];
        
        let strategy = CompositeBackoff::new(strategies);
        
        // First strategy (constant)
        assert_eq!(strategy.next_backoff(1), Some(Duration::from_millis(100)));
        assert_eq!(strategy.next_backoff(2), Some(Duration::from_millis(100)));
        
        // Second strategy (linear)
        assert_eq!(strategy.next_backoff(3), Some(Duration::from_millis(200)));
        assert_eq!(strategy.next_backoff(4), Some(Duration::from_millis(250)));
        
        // Beyond max attempts
        assert_eq!(strategy.next_backoff(5), None);
    }
    
    #[test]
    fn test_retry_policy() {
        let mut policy = RetryPolicy::constant(Duration::from_millis(100), 3);
        
        assert_eq!(policy.current_attempt(), 0);
        
        // First failure
        assert!(policy.record_failure());
        assert_eq!(policy.current_attempt(), 1);
        
        // Second failure
        assert!(policy.record_failure());
        assert_eq!(policy.current_attempt(), 2);
        
        // Third failure
        assert!(policy.record_failure());
        assert_eq!(policy.current_attempt(), 3);
        
        // Fourth failure (exceeds max)
        assert!(!policy.record_failure());
        assert_eq!(policy.current_attempt(), 4);
        
        // Reset
        policy.reset();
        assert_eq!(policy.current_attempt(), 0);
    }
}