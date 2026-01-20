//! Enhanced schema registry with version history and persistence.
//!
//! Extends the base schema registry with version tracking, persistence,
//! and schema family management.

use super::version::SchemaVersion;
use crate::error::{Result, XervError};
use crate::traits::{FieldInfo, TypeInfo};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Enhanced schema registry with version history.
///
/// Provides storage and lookup for schema metadata with support for:
/// - Version history tracking per schema family
/// - Hash-based lookup for WAL replay
/// - Optional persistence to JSON
/// - Compatibility checking between versions
#[derive(Default)]
pub struct VersionedSchemaRegistry {
    /// Schemas by full name (e.g., "OrderInput@v1").
    schemas: RwLock<HashMap<String, TypeInfo>>,
    /// Schemas by hash for fast lookup.
    by_hash: RwLock<HashMap<u64, String>>,
    /// Version history per schema family (short name -> sorted versions).
    versions: RwLock<HashMap<String, Vec<(SchemaVersion, u64)>>>,
    /// Persistence path (optional).
    persistence_path: Option<PathBuf>,
}

impl VersionedSchemaRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            schemas: RwLock::new(HashMap::new()),
            by_hash: RwLock::new(HashMap::new()),
            versions: RwLock::new(HashMap::new()),
            persistence_path: None,
        }
    }

    /// Create a registry with persistence.
    pub fn with_persistence(path: impl AsRef<Path>) -> Self {
        Self {
            schemas: RwLock::new(HashMap::new()),
            by_hash: RwLock::new(HashMap::new()),
            versions: RwLock::new(HashMap::new()),
            persistence_path: Some(path.as_ref().to_path_buf()),
        }
    }

    /// Register a schema.
    pub fn register(&self, info: TypeInfo) {
        let version = SchemaVersion::new(info.version as u16, 0);
        let hash = info.hash;

        let mut schemas = self.schemas.write();
        let mut by_hash = self.by_hash.write();
        let mut versions = self.versions.write();

        schemas.insert(info.name.clone(), info.clone());
        by_hash.insert(hash, info.name.clone());

        // Update version history
        let family_versions = versions.entry(info.short_name.clone()).or_default();
        if !family_versions.iter().any(|(v, _)| *v == version) {
            family_versions.push((version, hash));
            family_versions.sort_by_key(|(v, _)| *v);
        }
    }

    /// Register a schema with explicit version.
    pub fn register_versioned(&self, mut info: TypeInfo, version: SchemaVersion) {
        // Update the name to include the version (use to_short_string for consistency)
        info.name = format!("{}@{}", info.short_name, version.to_short_string());
        info.version = version.major as u32;

        self.register(info);
    }

    /// Get a schema by full name.
    pub fn get(&self, name: &str) -> Option<TypeInfo> {
        let schemas = self.schemas.read();
        schemas.get(name).cloned()
    }

    /// Get a schema by hash.
    pub fn get_by_hash(&self, hash: u64) -> Option<TypeInfo> {
        let by_hash = self.by_hash.read();
        let schemas = self.schemas.read();

        by_hash
            .get(&hash)
            .and_then(|name| schemas.get(name).cloned())
    }

    /// Get all versions of a schema family.
    pub fn get_versions(&self, short_name: &str) -> Vec<SchemaVersion> {
        let versions = self.versions.read();
        versions
            .get(short_name)
            .map(|v| v.iter().map(|(ver, _)| *ver).collect())
            .unwrap_or_default()
    }

    /// Get the latest version of a schema.
    pub fn get_latest(&self, short_name: &str) -> Option<TypeInfo> {
        let versions = self.versions.read();
        let schemas = self.schemas.read();

        versions.get(short_name).and_then(|v| {
            v.last().and_then(|(ver, _)| {
                // Use to_short_string() to match the format used by TypeInfo::new()
                let name = format!("{}@{}", short_name, ver.to_short_string());
                schemas.get(&name).cloned()
            })
        })
    }

    /// Get a specific version of a schema.
    pub fn get_version(&self, short_name: &str, version: SchemaVersion) -> Option<TypeInfo> {
        // Use to_short_string() to match the format used by TypeInfo::new()
        let name = format!("{}@{}", short_name, version.to_short_string());
        self.get(&name)
    }

    /// Check if a schema is registered.
    pub fn contains(&self, name: &str) -> bool {
        let schemas = self.schemas.read();
        schemas.contains_key(name)
    }

    /// Check if a schema family exists.
    pub fn contains_family(&self, short_name: &str) -> bool {
        let versions = self.versions.read();
        versions.contains_key(short_name)
    }

    /// Get all registered schema names.
    pub fn names(&self) -> Vec<String> {
        let schemas = self.schemas.read();
        schemas.keys().cloned().collect()
    }

    /// Get all schema families.
    pub fn families(&self) -> Vec<String> {
        let versions = self.versions.read();
        versions.keys().cloned().collect()
    }

    /// Check compatibility between two schemas.
    pub fn check_compatibility(&self, from: &str, to: &str) -> bool {
        let schemas = self.schemas.read();
        match (schemas.get(from), schemas.get(to)) {
            (Some(from_info), Some(to_info)) => from_info.is_compatible_with(to_info),
            _ => false,
        }
    }

    /// Persist the registry to disk.
    pub fn persist(&self) -> Result<()> {
        let path = self
            .persistence_path
            .as_ref()
            .ok_or_else(|| XervError::ConfigValue {
                field: "persistence_path".to_string(),
                cause: "No persistence path configured".to_string(),
            })?;

        let schemas = self.schemas.read();
        let versions = self.versions.read();

        let data = RegistryData {
            schemas: schemas.values().map(SchemaData::from).collect(),
            families: versions
                .iter()
                .map(|(name, vers)| FamilyData {
                    name: name.clone(),
                    versions: vers.iter().map(|(v, h)| (v.to_string(), *h)).collect(),
                })
                .collect(),
        };

        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| XervError::Serialization(e.to_string()))?;

        std::fs::write(path, json).map_err(|e| XervError::Io {
            path: path.clone(),
            cause: e.to_string(),
        })?;

        Ok(())
    }

    /// Load the registry from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        let json = std::fs::read_to_string(path).map_err(|e| XervError::Io {
            path: path.to_path_buf(),
            cause: e.to_string(),
        })?;

        let data: RegistryData =
            serde_json::from_str(&json).map_err(|e| XervError::Serialization(e.to_string()))?;

        let registry = Self::with_persistence(path);

        for schema_data in data.schemas {
            let info = schema_data.into_type_info();
            registry.register(info);
        }

        Ok(registry)
    }

    /// Clear all registered schemas.
    pub fn clear(&self) {
        let mut schemas = self.schemas.write();
        let mut by_hash = self.by_hash.write();
        let mut versions = self.versions.write();

        schemas.clear();
        by_hash.clear();
        versions.clear();
    }

    /// Get the total number of registered schemas.
    pub fn len(&self) -> usize {
        let schemas = self.schemas.read();
        schemas.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        let schemas = self.schemas.read();
        schemas.is_empty()
    }
}

/// Serializable schema data for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SchemaData {
    name: String,
    short_name: String,
    version: u32,
    hash: u64,
    size: usize,
    alignment: usize,
    fields: Vec<FieldData>,
    stable_layout: bool,
}

impl SchemaData {
    fn from(info: &TypeInfo) -> Self {
        Self {
            name: info.name.clone(),
            short_name: info.short_name.clone(),
            version: info.version,
            hash: info.hash,
            size: info.size,
            alignment: info.alignment,
            fields: info.fields.iter().map(FieldData::from).collect(),
            stable_layout: info.stable_layout,
        }
    }

    fn into_type_info(self) -> TypeInfo {
        TypeInfo {
            name: self.name,
            short_name: self.short_name,
            version: self.version,
            hash: self.hash,
            size: self.size,
            alignment: self.alignment,
            fields: self
                .fields
                .into_iter()
                .map(|f| f.into_field_info())
                .collect(),
            stable_layout: self.stable_layout,
        }
    }
}

/// Serializable field data for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FieldData {
    name: String,
    type_name: String,
    offset: usize,
    size: usize,
    optional: bool,
}

impl FieldData {
    fn from(info: &FieldInfo) -> Self {
        Self {
            name: info.name.clone(),
            type_name: info.type_name.clone(),
            offset: info.offset,
            size: info.size,
            optional: info.optional,
        }
    }

    fn into_field_info(self) -> FieldInfo {
        let mut info = FieldInfo::new(self.name, self.type_name)
            .with_offset(self.offset)
            .with_size(self.size);
        if self.optional {
            info = info.optional();
        }
        info
    }
}

/// Serializable family data for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FamilyData {
    name: String,
    versions: Vec<(String, u64)>,
}

/// Top-level registry persistence format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RegistryData {
    schemas: Vec<SchemaData>,
    families: Vec<FamilyData>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
