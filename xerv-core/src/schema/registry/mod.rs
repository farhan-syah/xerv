//! Enhanced schema registry with version history and persistence.
//!
//! Extends the base schema registry with version tracking, persistence,
//! and schema family management.

mod persistence;
mod versioned;

pub use versioned::VersionedSchemaRegistry;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::version::SchemaVersion;
    use crate::traits::{FieldInfo, TypeInfo};

    fn create_test_schema(name: &str, version: u32, hash: u64) -> TypeInfo {
        TypeInfo::new(name, version)
            .with_hash(hash)
            .with_size(32)
            .with_fields(vec![
                FieldInfo::new("id", "String").with_offset(0).with_size(24),
                FieldInfo::new("value", "i32").with_offset(24).with_size(4),
            ])
            .stable()
    }

    #[test]
    fn register_and_get() {
        let registry = VersionedSchemaRegistry::new();

        let schema = create_test_schema("Order", 1, 100);
        registry.register(schema);

        assert!(registry.contains("Order@v1"));
        assert!(!registry.contains("Order@v2"));

        let retrieved = registry.get("Order@v1").unwrap();
        assert_eq!(retrieved.hash, 100);
    }

    #[test]
    fn get_by_hash() {
        let registry = VersionedSchemaRegistry::new();

        let schema = create_test_schema("Order", 1, 12345);
        registry.register(schema);

        let retrieved = registry.get_by_hash(12345).unwrap();
        assert_eq!(retrieved.short_name, "Order");
    }

    #[test]
    fn version_history() {
        let registry = VersionedSchemaRegistry::new();

        registry.register(create_test_schema("Order", 1, 100));
        registry.register(create_test_schema("Order", 2, 200));
        registry.register(create_test_schema("Order", 3, 300));

        let versions = registry.get_versions("Order");
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0], SchemaVersion::new(1, 0));
        assert_eq!(versions[1], SchemaVersion::new(2, 0));
        assert_eq!(versions[2], SchemaVersion::new(3, 0));
    }

    #[test]
    fn get_latest() {
        let registry = VersionedSchemaRegistry::new();

        registry.register(create_test_schema("Order", 1, 100));
        registry.register(create_test_schema("Order", 2, 200));

        let latest = registry.get_latest("Order").unwrap();
        assert_eq!(latest.version, 2);
        assert_eq!(latest.hash, 200);
    }

    #[test]
    fn get_version() {
        let registry = VersionedSchemaRegistry::new();

        registry.register(create_test_schema("Order", 1, 100));
        registry.register(create_test_schema("Order", 2, 200));

        let v1 = registry.get_version("Order", SchemaVersion::new(1, 0));
        assert!(v1.is_some());
        assert_eq!(v1.unwrap().hash, 100);

        let v3 = registry.get_version("Order", SchemaVersion::new(3, 0));
        assert!(v3.is_none());
    }

    #[test]
    fn families() {
        let registry = VersionedSchemaRegistry::new();

        registry.register(create_test_schema("Order", 1, 100));
        registry.register(create_test_schema("User", 1, 200));

        let families = registry.families();
        assert_eq!(families.len(), 2);
        assert!(families.contains(&"Order".to_string()));
        assert!(families.contains(&"User".to_string()));
    }

    #[test]
    fn contains_family() {
        let registry = VersionedSchemaRegistry::new();

        registry.register(create_test_schema("Order", 1, 100));

        assert!(registry.contains_family("Order"));
        assert!(!registry.contains_family("User"));
    }

    #[test]
    fn clear() {
        let registry = VersionedSchemaRegistry::new();

        registry.register(create_test_schema("Order", 1, 100));
        assert!(!registry.is_empty());

        registry.clear();
        assert!(registry.is_empty());
        assert!(registry.families().is_empty());
    }

    #[test]
    fn persistence_roundtrip() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_schema_registry.json");

        // Create and populate registry
        let registry = VersionedSchemaRegistry::with_persistence(&path);
        registry.register(create_test_schema("Order", 1, 100));
        registry.register(create_test_schema("Order", 2, 200));
        registry.register(create_test_schema("User", 1, 300));

        // Persist
        registry.persist().unwrap();

        // Load into new registry
        let loaded = VersionedSchemaRegistry::load(&path).unwrap();

        // Verify
        assert_eq!(loaded.len(), 3);
        assert!(loaded.contains("Order@v1"));
        assert!(loaded.contains("Order@v2"));
        assert!(loaded.contains("User@v1"));
        assert_eq!(loaded.get_by_hash(200).unwrap().short_name, "Order");

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persistence_atomic_write() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_atomic_registry.json");
        let temp_path = path.with_extension("json.tmp");

        // Clean up any previous test artifacts
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&temp_path);

        // Create and persist registry
        let registry = VersionedSchemaRegistry::with_persistence(&path);
        registry.register(create_test_schema("Test", 1, 999));
        registry.persist().unwrap();

        // Verify the final file exists and temp file is cleaned up
        assert!(path.exists(), "Final file should exist");
        assert!(!temp_path.exists(), "Temp file should be cleaned up");

        // Verify we can still load it
        let loaded = VersionedSchemaRegistry::load(&path).unwrap();
        assert!(loaded.contains("Test@v1"));

        // Cleanup
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persistence_multiple_writes() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_multi_registry.json");

        // First write
        let registry1 = VersionedSchemaRegistry::with_persistence(&path);
        registry1.register(create_test_schema("First", 1, 111));
        registry1.persist().unwrap();

        // Second write (should overwrite)
        let registry2 = VersionedSchemaRegistry::with_persistence(&path);
        registry2.register(create_test_schema("Second", 1, 222));
        registry2.persist().unwrap();

        // Load and verify only second write is present
        let loaded = VersionedSchemaRegistry::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains("Second@v1"));
        assert!(!loaded.contains("First@v1"));

        // Cleanup
        let _ = std::fs::remove_file(path);
    }
}
