//! Schema trait and registry for type-safe data contracts.

use crate::error::{Result, XervError};
use std::collections::HashMap;
use std::sync::RwLock;

/// Information about a field in a schema.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Field name.
    pub name: String,
    /// Field type name.
    pub type_name: String,
    /// Byte offset within the struct (for stable layouts).
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// Whether the field is optional.
    pub optional: bool,
}

impl FieldInfo {
    /// Create a new field info.
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            offset: 0,
            size: 0,
            optional: false,
        }
    }

    /// Set the offset.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Set the size.
    pub fn with_size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    /// Mark as optional.
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Information about a type schema.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// Schema name with version (e.g., "OrderInput@v1").
    pub name: String,
    /// Short name without version.
    pub short_name: String,
    /// Version number.
    pub version: u32,
    /// Hash for quick comparison.
    pub hash: u64,
    /// Total size in bytes.
    pub size: usize,
    /// Alignment requirement.
    pub alignment: usize,
    /// Fields in the schema.
    pub fields: Vec<FieldInfo>,
    /// Whether this schema has a stable layout (#[repr(C)]).
    pub stable_layout: bool,
}

impl TypeInfo {
    /// Create a new type info.
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        let short_name = name.into();
        let full_name = format!("{}@v{}", short_name, version);

        Self {
            name: full_name,
            short_name,
            version,
            hash: 0,
            size: 0,
            alignment: 8,
            fields: Vec::new(),
            stable_layout: false,
        }
    }

    /// Set the hash.
    pub fn with_hash(mut self, hash: u64) -> Self {
        self.hash = hash;
        self
    }

    /// Set the size.
    pub fn with_size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    /// Set the alignment.
    pub fn with_alignment(mut self, alignment: usize) -> Self {
        self.alignment = alignment;
        self
    }

    /// Add a field.
    pub fn with_field(mut self, field: FieldInfo) -> Self {
        self.fields.push(field);
        self
    }

    /// Add multiple fields.
    pub fn with_fields(mut self, fields: Vec<FieldInfo>) -> Self {
        self.fields = fields;
        self
    }

    /// Mark as having a stable layout.
    pub fn stable(mut self) -> Self {
        self.stable_layout = true;
        self
    }

    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Check if this schema is compatible with another.
    ///
    /// A schema is forward-compatible if:
    /// - All required fields in `other` exist in `self`
    /// - Field types match
    pub fn is_compatible_with(&self, other: &TypeInfo) -> bool {
        // Same hash means identical
        if self.hash != 0 && self.hash == other.hash {
            return true;
        }

        // Check all required fields
        for field in &other.fields {
            if field.optional {
                continue;
            }

            match self.get_field(&field.name) {
                Some(our_field) => {
                    if our_field.type_name != field.type_name {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    }
}

/// The trait for types with schema metadata.
///
/// Types implementing this trait can be validated and introspected at runtime.
/// The `#[xerv::schema]` macro generates this implementation automatically.
pub trait Schema {
    /// Get the type information for this schema.
    fn type_info() -> TypeInfo;

    /// Get the schema hash.
    fn schema_hash() -> u64 {
        Self::type_info().hash
    }

    /// Validate that this type has a stable layout.
    fn validate_layout() -> Result<()> {
        let info = Self::type_info();
        if !info.stable_layout {
            return Err(XervError::NonDeterministicLayout {
                type_name: info.name,
                cause: "Type must use #[repr(C)] for stable memory layout".to_string(),
            });
        }
        Ok(())
    }
}

/// Registry for schema metadata.
///
/// The registry stores type information for all known schemas,
/// enabling runtime introspection and validation.
pub struct SchemaRegistry {
    /// Registered schemas by name.
    schemas: RwLock<HashMap<String, TypeInfo>>,
}

impl SchemaRegistry {
    /// Create a new schema registry.
    pub fn new() -> Self {
        Self {
            schemas: RwLock::new(HashMap::new()),
        }
    }

    /// Register a schema.
    pub fn register(&self, info: TypeInfo) {
        let mut schemas = self.schemas.write().unwrap();
        schemas.insert(info.name.clone(), info);
    }

    /// Get a schema by name.
    pub fn get(&self, name: &str) -> Option<TypeInfo> {
        let schemas = self.schemas.read().unwrap();
        schemas.get(name).cloned()
    }

    /// Get a schema by hash.
    pub fn get_by_hash(&self, hash: u64) -> Option<TypeInfo> {
        let schemas = self.schemas.read().unwrap();
        schemas.values().find(|t| t.hash == hash).cloned()
    }

    /// Check if a schema is registered.
    pub fn contains(&self, name: &str) -> bool {
        let schemas = self.schemas.read().unwrap();
        schemas.contains_key(name)
    }

    /// Get all registered schema names.
    pub fn names(&self) -> Vec<String> {
        let schemas = self.schemas.read().unwrap();
        schemas.keys().cloned().collect()
    }

    /// Check compatibility between two schemas by name.
    pub fn check_compatibility(&self, from: &str, to: &str) -> bool {
        let schemas = self.schemas.read().unwrap();
        match (schemas.get(from), schemas.get(to)) {
            (Some(from_info), Some(to_info)) => from_info.is_compatible_with(to_info),
            _ => false,
        }
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global schema registry.
///
/// This is intended for use by external code that needs a singleton registry.
/// Most internal code should create and manage its own `SchemaRegistry` instance.
#[allow(dead_code)]
static GLOBAL_REGISTRY: std::sync::OnceLock<SchemaRegistry> = std::sync::OnceLock::new();

/// Get the global schema registry.
///
/// Returns a reference to the global singleton `SchemaRegistry`. The registry
/// is lazily initialized on first access.
///
/// # Example
///
/// ```ignore
/// use xerv_core::traits::schema::{global_registry, TypeInfo};
///
/// let info = TypeInfo::new("MyType", 1);
/// global_registry().register(info);
/// ```
#[allow(dead_code)]
pub fn global_registry() -> &'static SchemaRegistry {
    GLOBAL_REGISTRY.get_or_init(SchemaRegistry::new)
}

/// Register a schema in the global registry.
///
/// Convenience function that registers a `TypeInfo` in the global singleton registry.
///
/// # Example
///
/// ```ignore
/// use xerv_core::traits::schema::{register_schema, TypeInfo};
///
/// let info = TypeInfo::new("Order", 1).with_hash(0x12345678);
/// register_schema(info);
/// ```
#[allow(dead_code)]
pub fn register_schema(info: TypeInfo) {
    global_registry().register(info);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_info_creation() {
        let info = TypeInfo::new("OrderInput", 1)
            .with_hash(0x12345678)
            .with_size(64)
            .with_fields(vec![
                FieldInfo::new("order_id", "String")
                    .with_offset(0)
                    .with_size(24),
                FieldInfo::new("amount", "f64").with_offset(24).with_size(8),
            ])
            .stable();

        assert_eq!(info.name, "OrderInput@v1");
        assert_eq!(info.short_name, "OrderInput");
        assert_eq!(info.version, 1);
        assert_eq!(info.fields.len(), 2);
        assert!(info.stable_layout);
    }

    #[test]
    fn schema_registry() {
        let registry = SchemaRegistry::new();

        let info1 = TypeInfo::new("TestType", 1).with_hash(111);
        let info2 = TypeInfo::new("TestType", 2).with_hash(222);

        registry.register(info1);
        registry.register(info2);

        assert!(registry.contains("TestType@v1"));
        assert!(registry.contains("TestType@v2"));
        assert!(!registry.contains("TestType@v3"));

        let retrieved = registry.get("TestType@v1").unwrap();
        assert_eq!(retrieved.hash, 111);
    }

    #[test]
    fn schema_compatibility() {
        let v1 = TypeInfo::new("Order", 1).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("amount", "f64"),
        ]);

        // v2 adds an optional field - should be compatible
        let v2_compatible = TypeInfo::new("Order", 2).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("amount", "f64"),
            FieldInfo::new("notes", "String").optional(),
        ]);

        assert!(v2_compatible.is_compatible_with(&v1));

        // v3 removes a required field - not compatible
        let v3_incompatible =
            TypeInfo::new("Order", 3).with_fields(vec![FieldInfo::new("id", "String")]);

        assert!(!v3_incompatible.is_compatible_with(&v1));
    }
}
