//! Arena compaction for reclaiming dead allocations.
//!
//! Compaction moves live data to eliminate gaps from dead allocations,
//! enabling the arena to reclaim space.

use super::allocation::{AllocationEntry, AllocationTracker};
use crate::error::{Result, XervError};
use crate::types::ArenaOffset;
use std::collections::HashMap;

/// Configuration for compaction.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Minimum fragmentation ratio to trigger compaction (0.0 to 1.0).
    pub fragmentation_threshold: f64,
    /// Minimum bytes that must be reclaimable to trigger compaction.
    pub min_reclaimable_bytes: u64,
    /// Whether to perform in-place compaction (true) or create a new arena.
    pub in_place: bool,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            fragmentation_threshold: 0.3,       // 30% fragmentation
            min_reclaimable_bytes: 1024 * 1024, // 1 MB
            in_place: true,
        }
    }
}

impl CompactionConfig {
    /// Set the fragmentation threshold.
    pub fn with_fragmentation_threshold(mut self, threshold: f64) -> Self {
        self.fragmentation_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Set the minimum reclaimable bytes.
    pub fn with_min_reclaimable_bytes(mut self, bytes: u64) -> Self {
        self.min_reclaimable_bytes = bytes;
        self
    }

    /// Set whether to perform in-place compaction.
    pub fn with_in_place(mut self, in_place: bool) -> Self {
        self.in_place = in_place;
        self
    }
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Map from old offsets to new offsets.
    pub offset_map: HashMap<ArenaOffset, ArenaOffset>,
    /// Bytes reclaimed by compaction.
    pub bytes_reclaimed: u64,
    /// Bytes still in use after compaction.
    pub bytes_used: u64,
    /// New generation number after compaction.
    pub new_generation: u32,
    /// Number of allocations moved.
    pub allocations_moved: usize,
    /// Whether compaction was performed.
    pub performed: bool,
}

impl CompactionResult {
    /// Create a result indicating no compaction was performed.
    pub fn not_performed() -> Self {
        Self {
            offset_map: HashMap::new(),
            bytes_reclaimed: 0,
            bytes_used: 0,
            new_generation: 0,
            allocations_moved: 0,
            performed: false,
        }
    }
}

/// Plan for compacting an arena.
#[derive(Debug)]
pub struct CompactionPlan {
    /// Allocations to move, with their new offsets.
    moves: Vec<(AllocationEntry, ArenaOffset)>,
    /// Total bytes to reclaim.
    bytes_to_reclaim: u64,
    /// New write position after compaction.
    new_write_pos: ArenaOffset,
}

impl CompactionPlan {
    /// Get the moves planned.
    pub fn moves(&self) -> &[(AllocationEntry, ArenaOffset)] {
        &self.moves
    }

    /// Get the bytes that will be reclaimed.
    pub fn bytes_to_reclaim(&self) -> u64 {
        self.bytes_to_reclaim
    }

    /// Get the new write position after compaction.
    pub fn new_write_pos(&self) -> ArenaOffset {
        self.new_write_pos
    }
}

/// Arena compactor for reclaiming space.
pub struct ArenaCompactor {
    /// Configuration for compaction.
    config: CompactionConfig,
}

impl ArenaCompactor {
    /// Create a new compactor with default configuration.
    pub fn new() -> Self {
        Self {
            config: CompactionConfig::default(),
        }
    }

    /// Create a compactor with custom configuration.
    pub fn with_config(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Check if compaction should be performed based on the tracker state.
    pub fn should_compact(&self, tracker: &AllocationTracker) -> bool {
        let fragmentation = tracker.fragmentation_ratio();
        let reclaimable = tracker.total_dead();

        fragmentation >= self.config.fragmentation_threshold
            && reclaimable >= self.config.min_reclaimable_bytes
    }

    /// Plan a compaction operation.
    ///
    /// This calculates where each live allocation should move to,
    /// without actually performing the move.
    pub fn plan(&self, tracker: &AllocationTracker, data_offset: ArenaOffset) -> CompactionPlan {
        let live_allocs = tracker.get_live_allocations();

        let mut moves = Vec::new();
        let mut current_pos = data_offset;

        for alloc in live_allocs {
            // Calculate aligned position
            let aligned_pos = align_offset(current_pos);

            // Check if this allocation needs to move
            if alloc.offset != aligned_pos {
                moves.push((alloc, aligned_pos));
            }

            // Calculate next position (with alignment)
            let alloc_size = align_size(alloc.size);
            current_pos = aligned_pos.add(alloc_size as u64);
        }

        CompactionPlan {
            moves,
            bytes_to_reclaim: tracker.total_dead(),
            new_write_pos: current_pos,
        }
    }

    /// Execute compaction on a memory buffer.
    ///
    /// This performs the actual data movement according to the plan.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - The buffer is large enough for all operations
    /// - No other code is reading/writing the affected regions
    pub fn execute(
        &self,
        tracker: &AllocationTracker,
        buffer: &mut [u8],
        data_offset: ArenaOffset,
    ) -> Result<CompactionResult> {
        if !self.should_compact(tracker) {
            return Ok(CompactionResult::not_performed());
        }

        let plan = self.plan(tracker, data_offset);

        // Build offset map
        let mut offset_map = HashMap::new();

        // Execute moves from end to start to avoid overwriting
        // data that hasn't been moved yet
        let mut moves_sorted: Vec<_> = plan.moves.clone();
        moves_sorted.sort_by(|a, b| b.0.offset.as_u64().cmp(&a.0.offset.as_u64()));

        for (alloc, new_offset) in &moves_sorted {
            let old_start = alloc.offset.as_u64() as usize;
            let new_start = new_offset.as_u64() as usize;
            let size = alloc.size as usize;

            // Validate bounds
            if old_start + size > buffer.len() || new_start + size > buffer.len() {
                return Err(XervError::CompactionFailed {
                    cause: format!(
                        "Move out of bounds: old={}, new={}, size={}, buffer_len={}",
                        old_start,
                        new_start,
                        size,
                        buffer.len()
                    ),
                });
            }

            // Copy data to new location
            // Use copy_within for overlapping regions
            if old_start < new_start {
                // Moving forward - copy from end
                for i in (0..size).rev() {
                    buffer[new_start + i] = buffer[old_start + i];
                }
            } else if old_start > new_start {
                // Moving backward - copy from start
                for i in 0..size {
                    buffer[new_start + i] = buffer[old_start + i];
                }
            }
            // If equal, no copy needed

            offset_map.insert(alloc.offset, *new_offset);
        }

        // Update the tracker
        let new_gen = tracker.increment_generation();

        // Rebuild allocations with new offsets
        let mut new_allocs = Vec::new();
        for alloc in tracker.get_live_allocations() {
            let new_offset = offset_map
                .get(&alloc.offset)
                .copied()
                .unwrap_or(alloc.offset);
            new_allocs.push(AllocationEntry {
                offset: new_offset,
                size: alloc.size,
                generation: new_gen,
                live: true,
            });
        }
        tracker.rebuild(new_allocs);

        let bytes_used = tracker.total_live();

        Ok(CompactionResult {
            offset_map,
            bytes_reclaimed: plan.bytes_to_reclaim,
            bytes_used,
            new_generation: new_gen,
            allocations_moved: moves_sorted.len(),
            performed: true,
        })
    }

    /// Get the configuration.
    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }
}

impl Default for ArenaCompactor {
    fn default() -> Self {
        Self::new()
    }
}

/// Align an offset to the entry alignment (8 bytes).
fn align_offset(offset: ArenaOffset) -> ArenaOffset {
    const ALIGNMENT: u64 = 8;
    let raw = offset.as_u64();
    let aligned = (raw + ALIGNMENT - 1) & !(ALIGNMENT - 1);
    ArenaOffset::new(aligned)
}

/// Align a size to the entry alignment (8 bytes).
fn align_size(size: u32) -> u32 {
    const ALIGNMENT: u32 = 8;
    (size + ALIGNMENT - 1) & !(ALIGNMENT - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_compact() {
        let compactor = ArenaCompactor::with_config(
            CompactionConfig::default()
                .with_fragmentation_threshold(0.3)
                .with_min_reclaimable_bytes(100),
        );
        let tracker = AllocationTracker::new();

        // No allocations - shouldn't compact
        assert!(!compactor.should_compact(&tracker));

        // Add allocations
        tracker.record_allocation(ArenaOffset::new(0x100), 100);
        tracker.record_allocation(ArenaOffset::new(0x200), 100);
        tracker.record_allocation(ArenaOffset::new(0x300), 100);

        // No dead allocations - shouldn't compact
        assert!(!compactor.should_compact(&tracker));

        // Mark one dead (33% fragmentation)
        tracker.mark_dead(ArenaOffset::new(0x200));

        // 33% >= 30%, 100 >= 100 - should compact
        assert!(compactor.should_compact(&tracker));
    }

    #[test]
    fn plan_compaction() {
        let compactor = ArenaCompactor::new();
        let tracker = AllocationTracker::new();

        // Allocations at 0x100, 0x200, 0x300 (each 64 bytes)
        tracker.record_allocation(ArenaOffset::new(0x100), 64);
        tracker.record_allocation(ArenaOffset::new(0x200), 64);
        tracker.record_allocation(ArenaOffset::new(0x300), 64);

        // Kill the middle one
        tracker.mark_dead(ArenaOffset::new(0x200));

        let plan = compactor.plan(&tracker, ArenaOffset::new(0x100));

        // First allocation stays at 0x100
        // Third allocation should move to 0x140 (0x100 + 64)
        assert_eq!(plan.moves.len(), 1);
        assert_eq!(plan.moves[0].0.offset.as_u64(), 0x300);
        assert_eq!(plan.moves[0].1.as_u64(), 0x140);
    }

    #[test]
    fn execute_compaction() {
        let compactor = ArenaCompactor::with_config(
            CompactionConfig::default()
                .with_fragmentation_threshold(0.2)
                .with_min_reclaimable_bytes(8),
        );
        let tracker = AllocationTracker::new();

        // Create a test buffer
        let mut buffer = vec![0u8; 0x400];

        // Write data at 0x100
        buffer[0x100..0x108].copy_from_slice(b"AAAAAAAA");
        tracker.record_allocation(ArenaOffset::new(0x100), 8);

        // Write data at 0x200 (will be dead)
        buffer[0x200..0x208].copy_from_slice(b"BBBBBBBB");
        tracker.record_allocation(ArenaOffset::new(0x200), 8);

        // Write data at 0x300
        buffer[0x300..0x308].copy_from_slice(b"CCCCCCCC");
        tracker.record_allocation(ArenaOffset::new(0x300), 8);

        // Kill middle allocation
        tracker.mark_dead(ArenaOffset::new(0x200));

        // Execute compaction
        let result = compactor
            .execute(&tracker, &mut buffer, ArenaOffset::new(0x100))
            .unwrap();

        assert!(result.performed);
        assert_eq!(result.allocations_moved, 1);
        assert_eq!(result.bytes_reclaimed, 8);

        // Verify data moved correctly
        // First allocation at 0x100 (unmoved)
        assert_eq!(&buffer[0x100..0x108], b"AAAAAAAA");
        // Third allocation moved to 0x108
        assert_eq!(&buffer[0x108..0x110], b"CCCCCCCC");
    }

    #[test]
    fn align_offset_test() {
        assert_eq!(align_offset(ArenaOffset::new(0)).as_u64(), 0);
        assert_eq!(align_offset(ArenaOffset::new(1)).as_u64(), 8);
        assert_eq!(align_offset(ArenaOffset::new(7)).as_u64(), 8);
        assert_eq!(align_offset(ArenaOffset::new(8)).as_u64(), 8);
        assert_eq!(align_offset(ArenaOffset::new(9)).as_u64(), 16);
    }

    #[test]
    fn align_size_test() {
        assert_eq!(align_size(0), 0);
        assert_eq!(align_size(1), 8);
        assert_eq!(align_size(7), 8);
        assert_eq!(align_size(8), 8);
        assert_eq!(align_size(9), 16);
    }
}
