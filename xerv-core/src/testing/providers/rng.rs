//! Random number generator provider for deterministic testing.
//!
//! Allows tests to use a seeded RNG for reproducible behavior.

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::time::{SystemTime, UNIX_EPOCH};

/// Provider trait for random number generation.
pub trait RngProvider: Send + Sync {
    /// Generate a random u64.
    fn next_u64(&self) -> u64;

    /// Generate a random f64 in the range [0, 1).
    fn next_f64(&self) -> f64;

    /// Fill a byte slice with random data.
    fn fill_bytes(&self, dest: &mut [u8]);

    /// Generate a random boolean with the given probability of being true.
    fn gen_bool(&self, probability: f64) -> bool {
        self.next_f64() < probability
    }

    /// Generate a random value in the given range.
    fn gen_range(&self, low: u64, high: u64) -> u64 {
        low + (self.next_u64() % (high - low))
    }

    /// Check if this is a mock provider.
    fn is_mock(&self) -> bool;
}

/// Real RNG that uses the system's entropy source.
///
/// Uses StdRng seeded from system time and entropy for thread-safety.
pub struct RealRng {
    rng: Mutex<StdRng>,
}

impl RealRng {
    /// Create a new real RNG.
    pub fn new() -> Self {
        // Seed from system time + entropy
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before UNIX epoch")
            .as_nanos() as u64;

        Self {
            rng: Mutex::new(StdRng::seed_from_u64(seed)),
        }
    }
}

impl Default for RealRng {
    fn default() -> Self {
        Self::new()
    }
}

impl RngProvider for RealRng {
    fn next_u64(&self) -> u64 {
        self.rng.lock().r#gen()
    }

    fn next_f64(&self) -> f64 {
        self.rng.lock().r#gen()
    }

    fn fill_bytes(&self, dest: &mut [u8]) {
        self.rng.lock().fill(dest);
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// Mock RNG with a fixed seed for deterministic behavior.
///
/// # Example
///
/// ```
/// use xerv_core::testing::MockRng;
/// use xerv_core::testing::RngProvider;
///
/// let rng = MockRng::seeded(42);
/// let first = rng.next_u64();
/// let second = rng.next_u64();
///
/// // Same seed produces same sequence
/// let rng2 = MockRng::seeded(42);
/// assert_eq!(rng2.next_u64(), first);
/// assert_eq!(rng2.next_u64(), second);
/// ```
pub struct MockRng {
    rng: Mutex<StdRng>,
    seed: u64,
}

impl MockRng {
    /// Create a new mock RNG with the given seed.
    pub fn seeded(seed: u64) -> Self {
        Self {
            rng: Mutex::new(StdRng::seed_from_u64(seed)),
            seed,
        }
    }

    /// Get the seed used to create this RNG.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Reset the RNG to its initial state.
    pub fn reset(&self) {
        *self.rng.lock() = StdRng::seed_from_u64(self.seed);
    }
}

impl RngProvider for MockRng {
    fn next_u64(&self) -> u64 {
        self.rng.lock().r#gen()
    }

    fn next_f64(&self) -> f64 {
        self.rng.lock().r#gen()
    }

    fn fill_bytes(&self, dest: &mut [u8]) {
        self.rng.lock().fill(dest);
    }

    fn is_mock(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_rng_is_deterministic() {
        let rng1 = MockRng::seeded(12345);
        let rng2 = MockRng::seeded(12345);

        let values1: Vec<u64> = (0..10).map(|_| rng1.next_u64()).collect();
        let values2: Vec<u64> = (0..10).map(|_| rng2.next_u64()).collect();

        assert_eq!(values1, values2);
    }

    #[test]
    fn mock_rng_reset() {
        let rng = MockRng::seeded(42);
        let first_run: Vec<u64> = (0..5).map(|_| rng.next_u64()).collect();

        rng.reset();
        let second_run: Vec<u64> = (0..5).map(|_| rng.next_u64()).collect();

        assert_eq!(first_run, second_run);
    }

    #[test]
    fn mock_rng_different_seeds_different_values() {
        let rng1 = MockRng::seeded(1);
        let rng2 = MockRng::seeded(2);

        let v1 = rng1.next_u64();
        let v2 = rng2.next_u64();

        assert_ne!(v1, v2);
    }

    #[test]
    fn mock_rng_gen_range() {
        let rng = MockRng::seeded(42);

        for _ in 0..100 {
            let v = rng.gen_range(10, 20);
            assert!((10..20).contains(&v));
        }
    }

    #[test]
    fn mock_rng_fill_bytes() {
        let rng = MockRng::seeded(42);

        let mut buf1 = [0u8; 16];
        let mut buf2 = [0u8; 16];

        rng.fill_bytes(&mut buf1);
        rng.reset();
        rng.fill_bytes(&mut buf2);

        assert_eq!(buf1, buf2);
    }
}
