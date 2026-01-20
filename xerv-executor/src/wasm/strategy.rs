//! Data transfer strategy selection for WASM nodes.
//!
//! Selects between copy-based transfer (for small payloads) and host API
//! access (for large payloads) based on payload size thresholds.

/// Threshold for switching from copy to host API strategy (1 MB).
const HOST_API_THRESHOLD: usize = 1024 * 1024;

/// Strategy for transferring data between host and WASM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStrategy {
    /// Copy bytes directly to WASM linear memory via `__xerv_alloc`.
    ///
    /// Best for payloads < 1 MB. Provides fastest access within WASM
    /// since data is local to the linear memory.
    SliceCopy,

    /// Use host API functions for data access.
    ///
    /// Best for payloads >= 1 MB. WASM calls host functions like
    /// `__xerv_read_bytes` to access data on demand, avoiding the
    /// cost of copying large buffers.
    HostApi,
}

impl TransferStrategy {
    /// Select the appropriate strategy based on payload size.
    ///
    /// # Arguments
    /// * `size` - Size of the payload in bytes
    ///
    /// # Returns
    /// - `SliceCopy` for payloads smaller than 1 MB
    /// - `HostApi` for payloads 1 MB or larger
    #[inline]
    pub fn select(size: usize) -> Self {
        if size < HOST_API_THRESHOLD {
            Self::SliceCopy
        } else {
            Self::HostApi
        }
    }

    /// Check if this strategy requires copying data to WASM memory.
    #[inline]
    pub fn requires_copy(self) -> bool {
        matches!(self, Self::SliceCopy)
    }

    /// Get the threshold size in bytes.
    #[inline]
    pub const fn threshold() -> usize {
        HOST_API_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_selection_small() {
        assert_eq!(TransferStrategy::select(0), TransferStrategy::SliceCopy);
        assert_eq!(TransferStrategy::select(1024), TransferStrategy::SliceCopy);
        assert_eq!(
            TransferStrategy::select(64 * 1024),
            TransferStrategy::SliceCopy
        );
        assert_eq!(
            TransferStrategy::select(HOST_API_THRESHOLD - 1),
            TransferStrategy::SliceCopy
        );
    }

    #[test]
    fn strategy_selection_large() {
        assert_eq!(
            TransferStrategy::select(HOST_API_THRESHOLD),
            TransferStrategy::HostApi
        );
        assert_eq!(
            TransferStrategy::select(HOST_API_THRESHOLD + 1),
            TransferStrategy::HostApi
        );
        assert_eq!(
            TransferStrategy::select(10 * 1024 * 1024),
            TransferStrategy::HostApi
        );
    }

    #[test]
    fn requires_copy() {
        assert!(TransferStrategy::SliceCopy.requires_copy());
        assert!(!TransferStrategy::HostApi.requires_copy());
    }

    #[test]
    fn threshold_value() {
        assert_eq!(TransferStrategy::threshold(), 1024 * 1024);
    }
}
