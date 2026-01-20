//! Chaos testing utilities.
//!
//! Provides tools for testing system resilience by injecting random failures,
//! latency, and other adverse conditions.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Configuration for chaos testing.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{ChaosConfig, ChaosFault};
///
/// let config = ChaosConfig::new()
///     .with_seed(42)
///     .with_fault_rate(0.1)  // 10% chance of fault
///     .with_max_latency_ms(500)
///     .with_fault(ChaosFault::NetworkTimeout { probability: 0.05 })
///     .with_fault(ChaosFault::ConnectionRefused { probability: 0.02 });
/// ```
#[derive(Debug, Clone)]
pub struct ChaosConfig {
    /// Base probability of injecting any fault (0.0 - 1.0).
    pub fault_rate: f64,
    /// Maximum latency to inject in milliseconds.
    pub max_latency_ms: u64,
    /// Specific faults to inject.
    pub faults: Vec<ChaosFault>,
    /// Random seed for reproducibility.
    pub seed: u64,
}

impl ChaosConfig {
    /// Create a new chaos configuration with no faults.
    pub fn new() -> Self {
        Self {
            fault_rate: 0.0,
            max_latency_ms: 0,
            faults: Vec::new(),
            seed: 0,
        }
    }

    /// Create a configuration with aggressive fault injection.
    ///
    /// Useful for stress testing.
    pub fn aggressive() -> Self {
        Self {
            fault_rate: 0.3,
            max_latency_ms: 1000,
            faults: vec![
                ChaosFault::NetworkTimeout { probability: 0.1 },
                ChaosFault::ConnectionRefused { probability: 0.1 },
                ChaosFault::SlowResponse {
                    probability: 0.2,
                    latency_ms: 500,
                },
            ],
            seed: 0,
        }
    }

    /// Create a configuration with mild fault injection.
    pub fn mild() -> Self {
        Self {
            fault_rate: 0.05,
            max_latency_ms: 200,
            faults: vec![
                ChaosFault::NetworkTimeout { probability: 0.01 },
                ChaosFault::SlowResponse {
                    probability: 0.05,
                    latency_ms: 100,
                },
            ],
            seed: 0,
        }
    }

    /// Set the random seed for reproducibility.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Set the base fault rate.
    pub fn with_fault_rate(mut self, rate: f64) -> Self {
        self.fault_rate = rate.clamp(0.0, 1.0);
        self
    }

    /// Set the maximum latency in milliseconds.
    pub fn with_max_latency_ms(mut self, ms: u64) -> Self {
        self.max_latency_ms = ms;
        self
    }

    /// Add a fault type.
    pub fn with_fault(mut self, fault: ChaosFault) -> Self {
        self.faults.push(fault);
        self
    }

    /// Clear all faults.
    pub fn without_faults(mut self) -> Self {
        self.faults.clear();
        self
    }
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Types of faults that can be injected.
#[derive(Debug, Clone)]
pub enum ChaosFault {
    /// Network timeout (request never completes).
    NetworkTimeout {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Connection refused error.
    ConnectionRefused {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Slow response (adds latency).
    SlowResponse {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
        /// Latency to add in milliseconds.
        latency_ms: u64,
    },

    /// Server error (HTTP 500).
    ServerError {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Rate limiting (HTTP 429).
    RateLimited {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Bad gateway (HTTP 502).
    BadGateway {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Service unavailable (HTTP 503).
    ServiceUnavailable {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Corrupted response data.
    CorruptedData {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Empty response.
    EmptyResponse {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
    },

    /// Custom fault with a specific HTTP status.
    CustomStatus {
        /// Probability of this fault (0.0 - 1.0).
        probability: f64,
        /// HTTP status code to return.
        status: u16,
    },
}

impl ChaosFault {
    /// Get the probability of this fault.
    pub fn probability(&self) -> f64 {
        match self {
            Self::NetworkTimeout { probability } => *probability,
            Self::ConnectionRefused { probability } => *probability,
            Self::SlowResponse { probability, .. } => *probability,
            Self::ServerError { probability } => *probability,
            Self::RateLimited { probability } => *probability,
            Self::BadGateway { probability } => *probability,
            Self::ServiceUnavailable { probability } => *probability,
            Self::CorruptedData { probability } => *probability,
            Self::EmptyResponse { probability } => *probability,
            Self::CustomStatus { probability, .. } => *probability,
        }
    }

    /// Get the name of this fault type.
    pub fn name(&self) -> &'static str {
        match self {
            Self::NetworkTimeout { .. } => "network_timeout",
            Self::ConnectionRefused { .. } => "connection_refused",
            Self::SlowResponse { .. } => "slow_response",
            Self::ServerError { .. } => "server_error",
            Self::RateLimited { .. } => "rate_limited",
            Self::BadGateway { .. } => "bad_gateway",
            Self::ServiceUnavailable { .. } => "service_unavailable",
            Self::CorruptedData { .. } => "corrupted_data",
            Self::EmptyResponse { .. } => "empty_response",
            Self::CustomStatus { .. } => "custom_status",
        }
    }
}

/// Engine for injecting chaos during tests.
///
/// Thread-safe and can be shared across async tasks.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{ChaosConfig, ChaosEngine, ChaosFault};
///
/// let config = ChaosConfig::new()
///     .with_seed(42)
///     .with_fault_rate(0.1)
///     .with_fault(ChaosFault::NetworkTimeout { probability: 0.05 });
///
/// let engine = ChaosEngine::new(config);
///
/// // Check if we should inject a fault
/// if engine.should_inject() {
///     if let Some(fault) = engine.select_fault() {
///         // Handle the fault
///     }
/// }
/// ```
pub struct ChaosEngine {
    config: ChaosConfig,
    rng: parking_lot::Mutex<StdRng>,
    injection_count: AtomicU64,
}

impl ChaosEngine {
    /// Create a new chaos engine with the given configuration.
    pub fn new(config: ChaosConfig) -> Self {
        let rng = StdRng::seed_from_u64(config.seed);
        Self {
            config,
            rng: parking_lot::Mutex::new(rng),
            injection_count: AtomicU64::new(0),
        }
    }

    /// Check if a fault should be injected based on the base fault rate.
    pub fn should_inject(&self) -> bool {
        if self.config.fault_rate <= 0.0 {
            return false;
        }
        self.rng.lock().r#gen::<f64>() < self.config.fault_rate
    }

    /// Select a fault to inject based on individual probabilities.
    ///
    /// Returns `None` if no fault should be injected.
    pub fn select_fault(&self) -> Option<ChaosFault> {
        if self.config.faults.is_empty() {
            return None;
        }

        let mut rng = self.rng.lock();

        // Try each fault based on its probability
        for fault in &self.config.faults {
            if rng.r#gen::<f64>() < fault.probability() {
                self.injection_count.fetch_add(1, Ordering::SeqCst);
                return Some(fault.clone());
            }
        }

        None
    }

    /// Get a random latency to inject.
    ///
    /// Returns a duration between 0 and `max_latency_ms`.
    pub fn random_latency(&self) -> Duration {
        if self.config.max_latency_ms == 0 {
            return Duration::ZERO;
        }
        let ms = self.rng.lock().gen_range(0..=self.config.max_latency_ms);
        Duration::from_millis(ms)
    }

    /// Get a specific latency percentage of the max.
    ///
    /// `factor` should be between 0.0 and 1.0.
    pub fn latency_factor(&self, factor: f64) -> Duration {
        let factor = factor.clamp(0.0, 1.0);
        let ms = (self.config.max_latency_ms as f64 * factor) as u64;
        Duration::from_millis(ms)
    }

    /// Get the number of faults injected so far.
    pub fn injection_count(&self) -> u64 {
        self.injection_count.load(Ordering::SeqCst)
    }

    /// Reset the injection count.
    pub fn reset_count(&self) {
        self.injection_count.store(0, Ordering::SeqCst);
    }

    /// Reset the RNG to its initial state.
    pub fn reset(&self) {
        *self.rng.lock() = StdRng::seed_from_u64(self.config.seed);
        self.reset_count();
    }

    /// Check if chaos injection is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.fault_rate > 0.0 || !self.config.faults.is_empty()
    }

    /// Get the configuration.
    pub fn config(&self) -> &ChaosConfig {
        &self.config
    }

    /// Convenience method: maybe inject latency.
    ///
    /// Returns the duration to sleep, or zero if no latency should be injected.
    pub fn maybe_latency(&self) -> Duration {
        if self.should_inject() {
            self.random_latency()
        } else {
            Duration::ZERO
        }
    }

    /// Convenience method: maybe fail with a fault.
    ///
    /// Returns `Ok(())` if no fault, or the fault if one should be injected.
    pub fn maybe_fail(&self) -> Result<(), ChaosFault> {
        if self.should_inject() {
            if let Some(fault) = self.select_fault() {
                return Err(fault);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaos_disabled_by_default() {
        let engine = ChaosEngine::new(ChaosConfig::new());

        // With 0% fault rate, should never inject
        for _ in 0..100 {
            assert!(!engine.should_inject());
        }
    }

    #[test]
    fn chaos_deterministic_with_seed() {
        let config = ChaosConfig::new().with_seed(42).with_fault_rate(0.5);

        let engine1 = ChaosEngine::new(config.clone());
        let engine2 = ChaosEngine::new(config);

        let results1: Vec<bool> = (0..20).map(|_| engine1.should_inject()).collect();
        let results2: Vec<bool> = (0..20).map(|_| engine2.should_inject()).collect();

        assert_eq!(results1, results2);
    }

    #[test]
    fn chaos_select_fault() {
        let config = ChaosConfig::new()
            .with_seed(42)
            .with_fault(ChaosFault::NetworkTimeout { probability: 0.5 })
            .with_fault(ChaosFault::ServerError { probability: 0.5 });

        let engine = ChaosEngine::new(config);

        // Run many times to get some faults
        let mut got_fault = false;
        for _ in 0..100 {
            if engine.select_fault().is_some() {
                got_fault = true;
                break;
            }
        }

        assert!(got_fault);
    }

    #[test]
    fn chaos_random_latency() {
        let config = ChaosConfig::new().with_seed(42).with_max_latency_ms(100);

        let engine = ChaosEngine::new(config);

        for _ in 0..10 {
            let latency = engine.random_latency();
            assert!(latency <= Duration::from_millis(100));
        }
    }

    #[test]
    fn chaos_reset() {
        let config = ChaosConfig::new().with_seed(42).with_fault_rate(0.5);

        let engine = ChaosEngine::new(config);

        let first_run: Vec<bool> = (0..10).map(|_| engine.should_inject()).collect();

        engine.reset();

        let second_run: Vec<bool> = (0..10).map(|_| engine.should_inject()).collect();

        assert_eq!(first_run, second_run);
    }

    #[test]
    fn chaos_injection_count() {
        let config = ChaosConfig::new()
            .with_seed(42)
            .with_fault(ChaosFault::NetworkTimeout { probability: 1.0 }); // 100% probability

        let engine = ChaosEngine::new(config);

        assert_eq!(engine.injection_count(), 0);

        engine.select_fault();
        assert_eq!(engine.injection_count(), 1);

        engine.select_fault();
        assert_eq!(engine.injection_count(), 2);

        engine.reset_count();
        assert_eq!(engine.injection_count(), 0);
    }

    #[test]
    fn chaos_aggressive_config() {
        let config = ChaosConfig::aggressive();

        assert_eq!(config.fault_rate, 0.3);
        assert_eq!(config.max_latency_ms, 1000);
        assert!(!config.faults.is_empty());
    }

    #[test]
    fn chaos_maybe_fail() {
        let config = ChaosConfig::new()
            .with_seed(42)
            .with_fault_rate(1.0) // Always inject
            .with_fault(ChaosFault::ServerError { probability: 1.0 }); // Always this fault

        let engine = ChaosEngine::new(config);

        let result = engine.maybe_fail();
        assert!(result.is_err());

        if let Err(fault) = result {
            assert_eq!(fault.name(), "server_error");
        }
    }
}
