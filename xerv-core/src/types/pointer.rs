//! Zero-copy pointer types for arena access.

use rkyv::{Archive, Deserialize, Serialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::fmt;
use std::marker::PhantomData;

/// Offset into the memory-mapped arena.
///
/// This is a raw byte offset from the start of the arena file.
/// Used internally for arena management.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Archive,
    Serialize,
    Deserialize,
    SerdeSerialize,
    SerdeDeserialize,
)]
#[rkyv(compare(PartialEq))]
#[repr(C)]
pub struct ArenaOffset(u64);

impl ArenaOffset {
    /// The null/invalid offset.
    pub const NULL: Self = Self(0);

    /// Create a new arena offset.
    #[must_use]
    pub const fn new(offset: u64) -> Self {
        Self(offset)
    }

    /// Get the raw offset value.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Check if this is the null offset.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    /// Add a byte offset.
    #[must_use]
    pub const fn add(&self, bytes: u64) -> Self {
        Self(self.0 + bytes)
    }

    /// Calculate the offset from another position (for relative pointers).
    #[must_use]
    pub const fn offset_from(&self, base: Self) -> i64 {
        self.0 as i64 - base.0 as i64
    }
}

impl fmt::Display for ArenaOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:08x}", self.0)
    }
}

impl From<u64> for ArenaOffset {
    fn from(offset: u64) -> Self {
        Self(offset)
    }
}

impl From<usize> for ArenaOffset {
    fn from(offset: usize) -> Self {
        Self(offset as u64)
    }
}

/// A relative pointer to data in the arena.
///
/// `RelPtr<T>` represents a pointer to data of type `T` stored in the arena.
/// The actual data is accessed through the arena, not directly through this pointer.
///
/// This enables zero-copy access where nodes can pass data by reference
/// rather than copying it.
#[derive(Archive, Serialize, Deserialize)]
#[repr(C)]
pub struct RelPtr<T> {
    /// The offset in the arena where the data starts.
    offset: ArenaOffset,
    /// The size in bytes of the archived data.
    size: u32,
    /// Phantom data for type safety.
    #[rkyv(with = rkyv::with::Skip)]
    _marker: PhantomData<T>,
}

impl<T> RelPtr<T> {
    /// The null/invalid relative pointer.
    pub const NULL: Self = Self {
        offset: ArenaOffset::NULL,
        size: 0,
        _marker: PhantomData,
    };

    /// Create a null/invalid pointer.
    #[must_use]
    pub const fn null() -> Self {
        Self::NULL
    }

    /// Create a new relative pointer.
    #[must_use]
    pub const fn new(offset: ArenaOffset, size: u32) -> Self {
        Self {
            offset,
            size,
            _marker: PhantomData,
        }
    }

    /// Get the arena offset.
    #[must_use]
    pub const fn offset(&self) -> ArenaOffset {
        self.offset
    }

    /// Get the size of the data in bytes.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// Check if this is a null pointer.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.offset.is_null()
    }

    /// Cast to a different type (unsafe, caller must ensure type compatibility).
    ///
    /// # Safety
    /// The caller must ensure that the data at this offset is actually
    /// a valid representation of type `U`.
    #[must_use]
    pub const unsafe fn cast<U>(self) -> RelPtr<U> {
        RelPtr {
            offset: self.offset,
            size: self.size,
            _marker: PhantomData,
        }
    }

    /// Create a typed pointer to a subfield.
    ///
    /// # Safety
    /// The caller must ensure that `field_offset` and `field_size` are valid
    /// for accessing a field of type `U` within the data.
    #[must_use]
    pub const unsafe fn field<U>(&self, field_offset: u32, field_size: u32) -> RelPtr<U> {
        RelPtr {
            offset: self.offset.add(field_offset as u64),
            size: field_size,
            _marker: PhantomData,
        }
    }
}

impl<T> Clone for RelPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for RelPtr<T> {}

impl<T> PartialEq for RelPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.offset == other.offset && self.size == other.size
    }
}

impl<T> Eq for RelPtr<T> {}

impl<T> std::hash::Hash for RelPtr<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.offset.hash(state);
        self.size.hash(state);
    }
}

impl<T> fmt::Debug for RelPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelPtr")
            .field("offset", &self.offset)
            .field("size", &self.size)
            .field("type", &std::any::type_name::<T>())
            .finish()
    }
}

impl<T> fmt::Display for RelPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RelPtr<{}>@{}[{}]",
            std::any::type_name::<T>(),
            self.offset,
            self.size
        )
    }
}

impl<T> Default for RelPtr<T> {
    fn default() -> Self {
        Self::NULL
    }
}

/// A slice of data in the arena.
///
/// Similar to `RelPtr<T>` but for contiguous arrays of type `T`.
#[derive(Archive, Serialize, Deserialize)]
#[repr(C)]
pub struct ArenaSlice<T> {
    /// The offset in the arena where the slice starts.
    offset: ArenaOffset,
    /// The number of elements in the slice.
    len: u32,
    /// The size of each element in bytes.
    element_size: u32,
    /// Phantom data for type safety.
    #[rkyv(with = rkyv::with::Skip)]
    _marker: PhantomData<T>,
}

impl<T> ArenaSlice<T> {
    /// An empty slice.
    pub const EMPTY: Self = Self {
        offset: ArenaOffset::NULL,
        len: 0,
        element_size: 0,
        _marker: PhantomData,
    };

    /// Create a new arena slice.
    #[must_use]
    pub const fn new(offset: ArenaOffset, len: u32, element_size: u32) -> Self {
        Self {
            offset,
            len,
            element_size,
            _marker: PhantomData,
        }
    }

    /// Get the arena offset.
    #[must_use]
    pub const fn offset(&self) -> ArenaOffset {
        self.offset
    }

    /// Get the number of elements.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// Check if the slice is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the size of each element.
    #[must_use]
    pub const fn element_size(&self) -> u32 {
        self.element_size
    }

    /// Get the total size in bytes.
    #[must_use]
    pub const fn total_size(&self) -> u64 {
        self.len as u64 * self.element_size as u64
    }

    /// Get a pointer to an element at the given index.
    ///
    /// Returns `None` if the index is out of bounds.
    #[must_use]
    pub fn get(&self, index: u32) -> Option<RelPtr<T>> {
        if index >= self.len {
            return None;
        }
        let element_offset = self.offset.add(index as u64 * self.element_size as u64);
        Some(RelPtr::new(element_offset, self.element_size))
    }
}

impl<T> Clone for ArenaSlice<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ArenaSlice<T> {}

impl<T> PartialEq for ArenaSlice<T> {
    fn eq(&self, other: &Self) -> bool {
        self.offset == other.offset
            && self.len == other.len
            && self.element_size == other.element_size
    }
}

impl<T> Eq for ArenaSlice<T> {}

impl<T> fmt::Debug for ArenaSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArenaSlice")
            .field("offset", &self.offset)
            .field("len", &self.len)
            .field("element_size", &self.element_size)
            .field("type", &std::any::type_name::<T>())
            .finish()
    }
}

impl<T> Default for ArenaSlice<T> {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_offset_basic() {
        let offset = ArenaOffset::new(0x100);
        assert_eq!(offset.as_u64(), 0x100);
        assert!(!offset.is_null());
        assert!(ArenaOffset::NULL.is_null());
    }

    #[test]
    fn arena_offset_add() {
        let offset = ArenaOffset::new(0x100);
        let new_offset = offset.add(0x50);
        assert_eq!(new_offset.as_u64(), 0x150);
    }

    #[test]
    fn arena_offset_display() {
        let offset = ArenaOffset::new(0x1234);
        assert_eq!(format!("{}", offset), "0x00001234");
    }

    #[test]
    fn rel_ptr_basic() {
        let ptr: RelPtr<u32> = RelPtr::new(ArenaOffset::new(0x100), 4);
        assert_eq!(ptr.offset().as_u64(), 0x100);
        assert_eq!(ptr.size(), 4);
        assert!(!ptr.is_null());
    }

    #[test]
    fn rel_ptr_null() {
        let ptr: RelPtr<u32> = RelPtr::NULL;
        assert!(ptr.is_null());
    }

    #[test]
    fn arena_slice_basic() {
        let slice: ArenaSlice<u64> = ArenaSlice::new(ArenaOffset::new(0x200), 10, 8);
        assert_eq!(slice.len(), 10);
        assert_eq!(slice.element_size(), 8);
        assert_eq!(slice.total_size(), 80);
    }

    #[test]
    fn arena_slice_get() {
        let slice: ArenaSlice<u64> = ArenaSlice::new(ArenaOffset::new(0x200), 10, 8);

        let elem0 = slice.get(0).unwrap();
        assert_eq!(elem0.offset().as_u64(), 0x200);

        let elem5 = slice.get(5).unwrap();
        assert_eq!(elem5.offset().as_u64(), 0x200 + 5 * 8);

        assert!(slice.get(10).is_none());
        assert!(slice.get(100).is_none());
    }
}
