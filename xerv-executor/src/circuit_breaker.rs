//! Circuit breaker for pipeline error rate management.
//!
//! Implements the circuit breaker pattern to prevent cascading failures when
//! pipelines experience high error rates.

use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation, requests are allowed.
    Closed,
    /// Circuit is tripped, requests are rejected.
    Open,
    /// Testing if service has recovered.
    HalfOpen,
}

/// Configuration for the circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before tripping the circuit.
    pub failure_threshold: u32,
    /// Time window for counting failures (in seconds).
    pub failure_window_secs: u64,
    /// How long to wait before testing recovery (in seconds).
    pub recovery_timeout_secs: u64,
    /// Number of successes in half-open state before closing.
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            failure_window_secs: 60,
            recovery_timeout_secs: 30,
            success_threshold: 3,
        }
    }
}

/// Circuit breaker for managing pipeline health.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<CircuitState>,
    /// Timestamps of recent failures within the window.
    failures: RwLock<VecDeque<Instant>>,
    /// Consecutive successes in half-open state.
    half_open_successes: AtomicU64,
    /// When the circuit was opened.
    opened_at: RwLock<Option<Instant>>,
    /// Total number of requests rejected.
    rejected_count: AtomicU64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(CircuitState::Closed),
            failures: RwLock::new(VecDeque::new()),
            half_open_successes: AtomicU64::new(0),
            opened_at: RwLock::new(None),
            rejected_count: AtomicU64::new(0),
        }
    }

    /// Create a circuit breaker with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Check if a request is allowed.
    ///
    /// Returns true if the request should proceed, false if it should be rejected.
    pub fn allow_request(&self) -> bool {
        let state = *self.state.read();

        match state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if recovery timeout has elapsed
                if let Some(opened_at) = *self.opened_at.read() {
                    if opened_at.elapsed() >= Duration::from_secs(self.config.recovery_timeout_secs)
                    {
                        // Transition to half-open
                        *self.state.write() = CircuitState::HalfOpen;
                        self.half_open_successes.store(0, Ordering::SeqCst);
                        return true;
                    }
                }
                self.rejected_count.fetch_add(1, Ordering::Relaxed);
                false
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record a successful request.
    pub fn record_success(&self) {
        let state = *self.state.read();

        if state == CircuitState::HalfOpen {
            let successes = self.half_open_successes.fetch_add(1, Ordering::SeqCst) + 1;
            if successes >= self.config.success_threshold as u64 {
                // Close the circuit
                *self.state.write() = CircuitState::Closed;
                *self.opened_at.write() = None;
                self.failures.write().clear();
            }
        }
    }

    /// Record a failed request.
    pub fn record_failure(&self) {
        let now = Instant::now();
        let window = Duration::from_secs(self.config.failure_window_secs);

        let mut failures = self.failures.write();

        // Remove old failures outside the window
        while let Some(&oldest) = failures.front() {
            if now.duration_since(oldest) > window {
                failures.pop_front();
            } else {
                break;
            }
        }

        // Add this failure
        failures.push_back(now);

        let state = *self.state.read();

        match state {
            CircuitState::Closed => {
                if failures.len() >= self.config.failure_threshold as usize {
                    // Trip the circuit
                    drop(failures);
                    *self.state.write() = CircuitState::Open;
                    *self.opened_at.write() = Some(now);
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open state trips the circuit again
                drop(failures);
                *self.state.write() = CircuitState::Open;
                *self.opened_at.write() = Some(now);
                self.half_open_successes.store(0, Ordering::SeqCst);
            }
            CircuitState::Open => {
                // Already open, nothing to do
            }
        }
    }

    /// Get the current circuit state.
    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }

    /// Get the number of rejected requests.
    pub fn rejected_count(&self) -> u64 {
        self.rejected_count.load(Ordering::Relaxed)
    }

    /// Get the number of failures in the current window.
    pub fn failure_count(&self) -> usize {
        let now = Instant::now();
        let window = Duration::from_secs(self.config.failure_window_secs);
        let failures = self.failures.read();

        failures
            .iter()
            .filter(|&&t| now.duration_since(t) <= window)
            .count()
    }

    /// Manually reset the circuit breaker to closed state.
    pub fn reset(&self) {
        *self.state.write() = CircuitState::Closed;
        *self.opened_at.write() = None;
        self.failures.write().clear();
        self.half_open_successes.store(0, Ordering::SeqCst);
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_closed() {
        let cb = CircuitBreaker::with_defaults();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn trips_after_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            failure_window_secs: 60,
            recovery_timeout_secs: 1,
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new(config);

        // Record failures
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Requests should be rejected
        assert!(!cb.allow_request());
        assert_eq!(cb.rejected_count(), 1);
    }

    #[test]
    fn recovers_after_timeout() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            failure_window_secs: 60,
            recovery_timeout_secs: 0, // Immediate recovery for testing
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new(config);

        // Trip the circuit
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // After timeout, should transition to half-open
        assert!(cb.allow_request());
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }

    #[test]
    fn closes_after_successes() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            failure_window_secs: 60,
            recovery_timeout_secs: 0,
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new(config);

        // Trip and recover to half-open
        cb.record_failure();
        cb.record_failure();
        cb.allow_request(); // Transitions to half-open

        // Record successes
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn failure_in_half_open_trips_again() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            failure_window_secs: 60,
            recovery_timeout_secs: 0,
            success_threshold: 3,
        };
        let cb = CircuitBreaker::new(config);

        // Trip and recover to half-open
        cb.record_failure();
        cb.record_failure();
        cb.allow_request(); // Transitions to half-open
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // A failure trips it again
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn manual_reset() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            failure_window_secs: 60,
            recovery_timeout_secs: 60,
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new(config);

        // Trip the circuit
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Manual reset
        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }
}
