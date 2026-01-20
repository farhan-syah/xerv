//! UUID provider for deterministic UUID generation.
//!
//! Allows tests to use predictable UUIDs for reproducible behavior.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

/// Provider trait for UUID generation.
pub trait UuidProvider: Send + Sync {
    /// Generate a new UUID.
    fn new_v4(&self) -> Uuid;

    /// Check if this is a mock provider.
    fn is_mock(&self) -> bool;
}

/// Real UUID provider that generates random UUIDs.
pub struct RealUuid;

impl RealUuid {
    /// Create a new real UUID provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealUuid {
    fn default() -> Self {
        Self::new()
    }
}

impl UuidProvider for RealUuid {
    fn new_v4(&self) -> Uuid {
        Uuid::new_v4()
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// Mock UUID provider for testing.
///
/// Can generate sequential UUIDs or return predetermined values.
pub struct MockUuid {
    mode: MockUuidMode,
}

enum MockUuidMode {
    /// Generate sequential UUIDs (00000000-0000-0000-0000-000000000001, etc.)
    Sequential(AtomicU64),
    /// Return predetermined UUIDs in order.
    Predetermined(Mutex<Vec<Uuid>>),
}

impl MockUuid {
    /// Create a mock UUID provider that generates sequential UUIDs.
    ///
    /// The first UUID will be 00000000-0000-0000-0000-000000000001.
    ///
    /// # Example
    ///
    /// ```
    /// use xerv_core::testing::MockUuid;
    /// use xerv_core::testing::UuidProvider;
    ///
    /// let provider = MockUuid::sequential();
    /// let id1 = provider.new_v4();
    /// let id2 = provider.new_v4();
    ///
    /// assert_eq!(id1.to_string(), "00000000-0000-0000-0000-000000000001");
    /// assert_eq!(id2.to_string(), "00000000-0000-0000-0000-000000000002");
    /// ```
    pub fn sequential() -> Self {
        Self {
            mode: MockUuidMode::Sequential(AtomicU64::new(1)),
        }
    }

    /// Create a mock UUID provider that generates sequential UUIDs starting at the given value.
    pub fn sequential_from(start: u64) -> Self {
        Self {
            mode: MockUuidMode::Sequential(AtomicU64::new(start)),
        }
    }

    /// Create a mock UUID provider that returns predetermined UUIDs.
    ///
    /// UUIDs are returned in the order provided. If more UUIDs are requested
    /// than provided, sequential UUIDs are generated starting from 1.
    ///
    /// # Example
    ///
    /// ```
    /// use xerv_core::testing::MockUuid;
    /// use xerv_core::testing::UuidProvider;
    /// use uuid::Uuid;
    ///
    /// let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
    /// let provider = MockUuid::predetermined(vec![id]);
    ///
    /// assert_eq!(provider.new_v4(), id);
    /// // Next call returns sequential UUID (starting from 1)
    /// assert_eq!(provider.new_v4().to_string(), "00000000-0000-0000-0000-000000000001");
    /// ```
    pub fn predetermined(uuids: Vec<Uuid>) -> Self {
        // Store in reverse order for efficient pop
        let mut reversed = uuids;
        reversed.reverse();
        Self {
            mode: MockUuidMode::Predetermined(Mutex::new(reversed)),
        }
    }

    /// Create a mock UUID provider from string representations.
    ///
    /// # Panics
    ///
    /// Panics if any string is not a valid UUID.
    pub fn from_strings(uuids: &[&str]) -> Self {
        let parsed: Vec<Uuid> = uuids
            .iter()
            .map(|s| Uuid::parse_str(s).expect("Invalid UUID string"))
            .collect();
        Self::predetermined(parsed)
    }

    /// Get the current sequential counter value (for debugging).
    pub fn current_counter(&self) -> Option<u64> {
        match &self.mode {
            MockUuidMode::Sequential(counter) => Some(counter.load(Ordering::SeqCst)),
            MockUuidMode::Predetermined(_) => None,
        }
    }
}

impl UuidProvider for MockUuid {
    fn new_v4(&self) -> Uuid {
        match &self.mode {
            MockUuidMode::Sequential(counter) => {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                // Create a UUID where the last 8 bytes are the counter
                let bytes: [u8; 16] = [
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    (n >> 56) as u8,
                    (n >> 48) as u8,
                    (n >> 40) as u8,
                    (n >> 32) as u8,
                    (n >> 24) as u8,
                    (n >> 16) as u8,
                    (n >> 8) as u8,
                    n as u8,
                ];
                Uuid::from_bytes(bytes)
            }
            MockUuidMode::Predetermined(uuids) => {
                let mut guard = uuids.lock();
                if let Some(uuid) = guard.pop() {
                    uuid
                } else {
                    // Fallback to sequential starting from current position
                    let n = guard.len() as u64 + 1;
                    drop(guard);
                    let bytes: [u8; 16] = [
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        (n >> 56) as u8,
                        (n >> 48) as u8,
                        (n >> 40) as u8,
                        (n >> 32) as u8,
                        (n >> 24) as u8,
                        (n >> 16) as u8,
                        (n >> 8) as u8,
                        n as u8,
                    ];
                    Uuid::from_bytes(bytes)
                }
            }
        }
    }

    fn is_mock(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_uuids() {
        let provider = MockUuid::sequential();

        let id1 = provider.new_v4();
        let id2 = provider.new_v4();
        let id3 = provider.new_v4();

        assert_eq!(id1.to_string(), "00000000-0000-0000-0000-000000000001");
        assert_eq!(id2.to_string(), "00000000-0000-0000-0000-000000000002");
        assert_eq!(id3.to_string(), "00000000-0000-0000-0000-000000000003");
    }

    #[test]
    fn sequential_from() {
        let provider = MockUuid::sequential_from(100);

        let id = provider.new_v4();
        assert_eq!(id.to_string(), "00000000-0000-0000-0000-000000000064"); // 100 in hex = 64
    }

    #[test]
    fn predetermined_uuids() {
        let id1 = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let id2 = Uuid::parse_str("abcdefab-cdef-abcd-efab-cdefabcdefab").unwrap();

        let provider = MockUuid::predetermined(vec![id1, id2]);

        assert_eq!(provider.new_v4(), id1);
        assert_eq!(provider.new_v4(), id2);
    }

    #[test]
    fn from_strings() {
        let provider = MockUuid::from_strings(&[
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
        ]);

        assert_eq!(
            provider.new_v4().to_string(),
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(
            provider.new_v4().to_string(),
            "22222222-2222-2222-2222-222222222222"
        );
    }

    #[test]
    fn real_uuid_is_random() {
        let provider = RealUuid::new();
        let id1 = provider.new_v4();
        let id2 = provider.new_v4();

        assert_ne!(id1, id2);
    }
}
