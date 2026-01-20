//! Memory-mapped append-only arena for zero-copy data sharing.
//!
//! The Arena is the core of XERV's data plane. Each trace gets its own
//! arena file (`/tmp/xerv/trace_{uuid}.bin`) where all node outputs are
//! stored. Nodes access data via relative pointers (`RelPtr<T>`) without
//! copying.
//!
//! # Layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ Header (fixed size, contains metadata + pipeline config offset) │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Pipeline Config (rkyv-serialized, read-only after init)         │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Data Region (append-only node outputs)                          │
//! │ ┌─────────────────────────────────────────────────────────────┐ │
//! │ │ Entry 0: [size: u32][data: bytes]                           │ │
//! │ ├─────────────────────────────────────────────────────────────┤ │
//! │ │ Entry 1: [size: u32][data: bytes]                           │ │
//! │ ├─────────────────────────────────────────────────────────────┤ │
//! │ │ ...                                                          │ │
//! │ └─────────────────────────────────────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Compaction
//!
//! The arena supports compaction to reclaim space from dead allocations.
//! Compaction is triggered when fragmentation exceeds a configurable
//! threshold. See the `allocation` and `compaction` modules for details.

mod allocation;
mod compaction;
mod header;
mod writer;

pub use allocation::{AllocationEntry, AllocationTracker};
pub use compaction::{ArenaCompactor, CompactionConfig, CompactionPlan, CompactionResult};
pub use header::ArenaHeader;
pub use writer::{Arena, ArenaConfig, ArenaReader, ArenaWriter};
