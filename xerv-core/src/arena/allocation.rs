//! Allocation tracking for arena garbage collection.
//!
//! Tracks all allocations in the arena to enable compaction.

use crate::types::ArenaOffset;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// An entry tracking a single allocation.
#[derive(Debug, Clone, Copy)]
pub struct AllocationEntry {
    /// Offset in the arena where the allocation starts.
    pub offset: ArenaOffset,
    /// Size of the allocation in bytes.
    pub size: u32,
    /// Generation when this allocation was made.
    pub generation: u32,
    /// Whether this allocation is still live (referenced).
    pub live: bool,
}

impl AllocationEntry {
    /// Create a new allocation entry.
    pub fn new(offset: ArenaOffset, size: u32, generation: u32) -> Self {
        Self {
            offset,
            size,
            generation,
            live: true,
        }
    }

    /// Mark this allocation as dead (no longer referenced).
    pub fn mark_dead(&mut self) {
        self.live = false;
    }

    /// Get the end offset of this allocation.
    pub fn end_offset(&self) -> ArenaOffset {
        self.offset.add(self.size as u64)
    }
}

/// Tracks all allocations in an arena for garbage collection.
pub struct AllocationTracker {
    /// Map from offset to allocation entry.
    allocations: RwLock<HashMap<ArenaOffset, AllocationEntry>>,
    /// Current generation number.
    generation: AtomicU32,
    /// Total bytes allocated.
    total_allocated: RwLock<u64>,
    /// Total bytes marked as dead (reclaimable).
    total_dead: RwLock<u64>,
}

impl AllocationTracker {
    /// Create a new allocation tracker.
    pub fn new() -> Self {
        Self {
            allocations: RwLock::new(HashMap::new()),
            generation: AtomicU32::new(1),
            total_allocated: RwLock::new(0),
            total_dead: RwLock::new(0),
        }
    }

    /// Record a new allocation.
    pub fn record_allocation(&self, offset: ArenaOffset, size: u32) {
        let generation = self.generation.load(Ordering::Relaxed);
        let entry = AllocationEntry::new(offset, size, generation);

        let mut allocs = self.allocations.write();
        allocs.insert(offset, entry);

        let mut total = self.total_allocated.write();
        *total += size as u64;
    }

    /// Mark an allocation as dead (eligible for reclamation).
    pub fn mark_dead(&self, offset: ArenaOffset) {
        let mut allocs = self.allocations.write();

        if let Some(entry) = allocs.get_mut(&offset) {
            if entry.live {
                entry.live = false;

                let mut dead = self.total_dead.write();
                *dead += entry.size as u64;
            }
        }
    }

    /// Check if an allocation is live.
    pub fn is_live(&self, offset: ArenaOffset) -> bool {
        self.allocations
            .read()
            .get(&offset)
            .map(|e| e.live)
            .unwrap_or(false)
    }

    /// Get all live allocations sorted by offset.
    pub fn get_live_allocations(&self) -> Vec<AllocationEntry> {
        let allocs = self.allocations.read();
        let mut live: Vec<_> = allocs.values().filter(|e| e.live).cloned().collect();
        live.sort_by_key(|e| e.offset.as_u64());
        live
    }

    /// Get all dead allocations sorted by offset.
    pub fn get_dead_allocations(&self) -> Vec<AllocationEntry> {
        let allocs = self.allocations.read();
        let mut dead: Vec<_> = allocs.values().filter(|e| !e.live).cloned().collect();
        dead.sort_by_key(|e| e.offset.as_u64());
        dead
    }

    /// Get the current generation.
    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Increment the generation (after compaction).
    pub fn increment_generation(&self) -> u32 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Get total bytes allocated.
    pub fn total_allocated(&self) -> u64 {
        *self.total_allocated.read()
    }

    /// Get total bytes that are dead (reclaimable).
    pub fn total_dead(&self) -> u64 {
        *self.total_dead.read()
    }

    /// Get total bytes that are live.
    pub fn total_live(&self) -> u64 {
        self.total_allocated() - self.total_dead()
    }

    /// Calculate fragmentation ratio (0.0 = no fragmentation, 1.0 = all dead).
    pub fn fragmentation_ratio(&self) -> f64 {
        let total = self.total_allocated();
        if total == 0 {
            return 0.0;
        }
        self.total_dead() as f64 / total as f64
    }

    /// Get the number of allocations.
    pub fn allocation_count(&self) -> usize {
        self.allocations.read().len()
    }

    /// Get the number of live allocations.
    pub fn live_count(&self) -> usize {
        self.allocations.read().values().filter(|e| e.live).count()
    }

    /// Clear all tracking (after compaction creates new arena).
    pub fn clear(&self) {
        let mut allocs = self.allocations.write();
        allocs.clear();

        let mut total = self.total_allocated.write();
        *total = 0;

        let mut dead = self.total_dead.write();
        *dead = 0;
    }

    /// Rebuild the tracker with a new set of allocations.
    pub fn rebuild(&self, allocations: Vec<AllocationEntry>) {
        let mut allocs = self.allocations.write();
        allocs.clear();

        let mut total_alloc = 0u64;
        let mut total_dead = 0u64;

        for entry in allocations {
            total_alloc += entry.size as u64;
            if !entry.live {
                total_dead += entry.size as u64;
            }
            allocs.insert(entry.offset, entry);
        }

        let mut total = self.total_allocated.write();
        *total = total_alloc;

        let mut dead = self.total_dead.write();
        *dead = total_dead;
    }

    /// Update an allocation's offset (after compaction).
    pub fn update_offset(&self, old_offset: ArenaOffset, new_offset: ArenaOffset) {
        let mut allocs = self.allocations.write();

        if let Some(mut entry) = allocs.remove(&old_offset) {
            entry.offset = new_offset;
            allocs.insert(new_offset, entry);
        }
    }
}

impl Default for AllocationTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_allocations() {
        let tracker = AllocationTracker::new();

        tracker.record_allocation(ArenaOffset::new(0x100), 64);
        tracker.record_allocation(ArenaOffset::new(0x150), 32);
        tracker.record_allocation(ArenaOffset::new(0x180), 128);

        assert_eq!(tracker.allocation_count(), 3);
        assert_eq!(tracker.live_count(), 3);
        assert_eq!(tracker.total_allocated(), 64 + 32 + 128);
        assert_eq!(tracker.total_dead(), 0);
    }

    #[test]
    fn mark_dead() {
        let tracker = AllocationTracker::new();

        tracker.record_allocation(ArenaOffset::new(0x100), 64);
        tracker.record_allocation(ArenaOffset::new(0x150), 32);

        tracker.mark_dead(ArenaOffset::new(0x100));

        assert_eq!(tracker.live_count(), 1);
        assert_eq!(tracker.total_dead(), 64);
        assert!(!tracker.is_live(ArenaOffset::new(0x100)));
        assert!(tracker.is_live(ArenaOffset::new(0x150)));
    }

    #[test]
    fn fragmentation_ratio() {
        let tracker = AllocationTracker::new();

        tracker.record_allocation(ArenaOffset::new(0x100), 100);
        tracker.record_allocation(ArenaOffset::new(0x200), 100);

        assert_eq!(tracker.fragmentation_ratio(), 0.0);

        tracker.mark_dead(ArenaOffset::new(0x100));

        assert_eq!(tracker.fragmentation_ratio(), 0.5);
    }

    #[test]
    fn get_live_allocations() {
        let tracker = AllocationTracker::new();

        tracker.record_allocation(ArenaOffset::new(0x200), 32);
        tracker.record_allocation(ArenaOffset::new(0x100), 64);
        tracker.record_allocation(ArenaOffset::new(0x150), 32);

        tracker.mark_dead(ArenaOffset::new(0x150));

        let live = tracker.get_live_allocations();
        assert_eq!(live.len(), 2);
        // Should be sorted by offset
        assert_eq!(live[0].offset.as_u64(), 0x100);
        assert_eq!(live[1].offset.as_u64(), 0x200);
    }

    #[test]
    fn generation() {
        let tracker = AllocationTracker::new();

        assert_eq!(tracker.generation(), 1);

        tracker.record_allocation(ArenaOffset::new(0x100), 64);
        assert_eq!(
            tracker
                .allocations
                .read()
                .get(&ArenaOffset::new(0x100))
                .unwrap()
                .generation,
            1
        );

        let new_gen = tracker.increment_generation();
        assert_eq!(new_gen, 2);
        assert_eq!(tracker.generation(), 2);

        tracker.record_allocation(ArenaOffset::new(0x200), 64);
        assert_eq!(
            tracker
                .allocations
                .read()
                .get(&ArenaOffset::new(0x200))
                .unwrap()
                .generation,
            2
        );
    }
}
