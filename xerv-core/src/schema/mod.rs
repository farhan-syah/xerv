//! Schema evolution system for XERV.
//!
//! This module provides infrastructure for safe, versioned schema changes
//! with backward compatibility, migration hooks, and runtime version negotiation.
//!
//! # Overview
//!
//! The schema evolution system enables:
//! - Semantic versioning of schemas (major.minor)
//! - Breaking vs non-breaking change classification
//! - Migration functions for transforming data between versions
//! - Compatibility checking between schema versions
//! - Default values for new fields during migration
//! - Persistent schema registry with version history
//!
//! # Core Components
//!
//! - [`SchemaVersion`] - Semantic version for schemas (major.minor)
//! - [`VersionRange`] - Version range for compatibility declarations
//! - [`ChangeKind`] - Classification of schema changes
//! - [`Migration`] - Transform function between schema versions
//! - [`MigrationRegistry`] - Registry of available migrations with path-finding
//! - [`CompatibilityMatrix`] - Schema comparison and change detection
//! - [`DefaultProvider`] - Default values for schema fields
//! - [`VersionedSchemaRegistry`] - Enhanced registry with version history
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::schema::*;
//!
//! // Define schema versions
//! let v1 = SchemaVersion::new(1, 0);
//! let v2 = SchemaVersion::new(2, 0);
//!
//! // Register a migration
//! let registry = MigrationRegistry::new();
//! registry.register(Migration::new(
//!     "OrderInput@v1",
//!     v1_hash,
//!     "OrderInput@v2",
//!     v2_hash,
//!     |arena, offset| {
//!         // Transform v1 data to v2 format
//!         Ok(offset) // placeholder
//!     },
//! ))?;
//!
//! // Check if migration is possible
//! if registry.has_path(v1_hash, v2_hash) {
//!     let new_offset = registry.migrate(arena, old_offset, v1_hash, v2_hash)?;
//! }
//! ```
//!
//! # Schema Compatibility
//!
//! Changes are classified as breaking or non-breaking:
//!
//! | Change Type | Breaking? | Notes |
//! |:------------|:----------|:------|
//! | Add optional field | No | Backward compatible |
//! | Add required field | Yes | Needs migration or default |
//! | Remove field | Yes | Needs migration |
//! | Change field type | Yes | Needs migration |
//! | Rename field | Yes | Needs migration |
//! | Make field optional | No | Backward compatible |
//! | Make field required | Yes | Needs migration or default |

mod compatibility;
mod defaults;
mod migration;
mod registry;
mod version;

// Re-export all public types
pub use compatibility::{CompatibilityMatrix, CompatibilityReport, SchemaChange};
pub use defaults::{DefaultProvider, DefaultRegistry, FieldDefault, MapDefaultProvider};
pub use migration::{Migration, MigrationFn, MigrationRegistry, compose_migrations};
pub use registry::VersionedSchemaRegistry;
pub use version::{ChangeKind, SchemaVersion, VersionRange};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{FieldInfo, TypeInfo};
    use std::sync::Arc;

    #[test]
    fn integration_test_schema_evolution() {
        // Create registries
        let migration_registry = Arc::new(MigrationRegistry::new());
        let schema_registry = VersionedSchemaRegistry::new();
        let default_registry = DefaultRegistry::new();

        // Define v1 schema
        let v1 = TypeInfo::new("Order", 1).with_hash(100).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("amount", "f64"),
        ]);

        // Define v2 schema (adds optional currency field)
        let v2 = TypeInfo::new("Order", 2).with_hash(200).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("amount", "f64"),
            FieldInfo::new("currency", "String").optional(),
        ]);

        // Register schemas
        schema_registry.register(v1.clone());
        schema_registry.register(v2.clone());

        // Register default for new field
        default_registry.register(
            "Order@v2",
            MapDefaultProvider::new().with_string("currency", "USD"),
        );

        // Check compatibility
        let compat_matrix = CompatibilityMatrix::new(Arc::clone(&migration_registry));
        let report = compat_matrix.check(&v1, &v2);

        // Adding optional field is non-breaking
        assert!(report.compatible);
        assert!(!report.migration_required);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(report.changes[0].kind, ChangeKind::AddOptionalField);

        // Verify version history
        let versions = schema_registry.get_versions("Order");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0], SchemaVersion::new(1, 0));
        assert_eq!(versions[1], SchemaVersion::new(2, 0));

        // Verify latest
        let latest = schema_registry.get_latest("Order").unwrap();
        assert_eq!(latest.version, 2);
    }

    #[test]
    fn integration_test_breaking_change_with_migration() {
        let migration_registry = Arc::new(MigrationRegistry::new());

        // Define v1 schema
        let v1 = TypeInfo::new("User", 1).with_hash(1000).with_fields(vec![
            FieldInfo::new("name", "String"),
            FieldInfo::new("age", "i32"),
        ]);

        // Define v2 schema (age changed to f64)
        let v2 = TypeInfo::new("User", 2).with_hash(2000).with_fields(vec![
            FieldInfo::new("name", "String"),
            FieldInfo::new("age", "f64"), // Changed type
        ]);

        // Without migration, it's incompatible
        let compat_matrix = CompatibilityMatrix::new(Arc::clone(&migration_registry));
        let report = compat_matrix.check(&v1, &v2);

        assert!(!report.compatible);
        assert!(report.migration_required);
        assert!(!report.migration_available);

        // Register a migration
        migration_registry
            .register(Migration::new(
                "User@v1",
                1000,
                "User@v2",
                2000,
                |_arena, offset| Ok(offset), // Placeholder
            ))
            .unwrap();

        // Now migration is available
        let report = compat_matrix.check(&v1, &v2);
        assert!(!report.compatible);
        assert!(report.migration_required);
        assert!(report.migration_available);
    }
}
