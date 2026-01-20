//! Versioned schema registry implementation.

use super::persistence::{FamilyData, RegistryData, SchemaData};
use crate::error::{Result, XervError};
use crate::schema::version::SchemaVersion;
use crate::traits::TypeInfo;
use fs2::FileExt;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
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
    ///
    /// This method uses exclusive file locking to prevent race conditions
    /// when multiple processes attempt to update the registry simultaneously.
    /// The lock is held only during the write operation.
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

        // Use atomic write pattern: write to temp file, then rename
        // Combined with exclusive file locking for cross-process safety
        let temp_path = path.with_extension("json.tmp");

        // Open/create temp file with exclusive lock
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)
            .map_err(|e| XervError::Io {
                path: temp_path.clone(),
                cause: format!("Failed to create temp file: {}", e),
            })?;

        // Acquire exclusive lock (blocks if another process has the lock)
        file.lock_exclusive().map_err(|e| XervError::Io {
            path: temp_path.clone(),
            cause: format!("Failed to acquire exclusive lock: {}", e),
        })?;

        // Write data
        file.write_all(json.as_bytes()).map_err(|e| XervError::Io {
            path: temp_path.clone(),
            cause: format!("Failed to write registry data: {}", e),
        })?;

        // Sync to disk before rename
        file.sync_all().map_err(|e| XervError::Io {
            path: temp_path.clone(),
            cause: format!("Failed to sync to disk: {}", e),
        })?;

        // Rename is atomic on most filesystems
        std::fs::rename(&temp_path, path).map_err(|e| XervError::Io {
            path: path.clone(),
            cause: format!("Failed to rename temp file: {}", e),
        })?;

        // Lock is released when file is dropped

        Ok(())
    }

    /// Load the registry from disk.
    ///
    /// This method uses shared file locking to allow concurrent reads
    /// while blocking writes during the read operation.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        // Open file for reading with shared lock
        let mut file = File::open(path).map_err(|e| XervError::Io {
            path: path.to_path_buf(),
            cause: format!("Failed to open registry file: {}", e),
        })?;

        // Acquire shared lock (allows other readers, blocks writers)
        // Use explicit fs2::FileExt call to avoid conflict with std lock_shared (1.89+)
        FileExt::lock_shared(&file).map_err(|e| XervError::Io {
            path: path.to_path_buf(),
            cause: format!("Failed to acquire shared lock: {}", e),
        })?;

        // Read contents
        let mut json = String::new();
        file.read_to_string(&mut json).map_err(|e| XervError::Io {
            path: path.to_path_buf(),
            cause: format!("Failed to read registry file: {}", e),
        })?;

        // Lock is released when file is dropped

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
