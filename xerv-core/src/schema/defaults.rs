//! Default value handling for schema fields.
//!
//! Provides infrastructure for defining and retrieving default values
//! for schema fields, enabling safe schema evolution with new fields.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Default value provider for schema fields.
///
/// Implementations provide default values for fields in a schema,
/// which are used when migrating data from older versions that
/// lack certain fields.
pub trait DefaultProvider: Send + Sync {
    /// Get the default value for a field as bytes.
    ///
    /// # Arguments
    ///
    /// * `field` - The field name
    /// * `type_name` - The field's type name
    ///
    /// # Returns
    ///
    /// The default value as bytes (for arena storage), or None if no default.
    fn default_bytes(&self, field: &str, type_name: &str) -> Option<Vec<u8>>;

    /// Get the default value for a field as a string representation.
    ///
    /// This is useful for debugging and display purposes.
    fn default_string(&self, field: &str, type_name: &str) -> Option<String>;

    /// Check if a field has a default value.
    fn has_default(&self, field: &str) -> bool;

    /// Get all fields that have defaults.
    fn fields_with_defaults(&self) -> Vec<String>;
}

/// A simple default provider backed by a map.
#[derive(Default)]
pub struct MapDefaultProvider {
    /// Field name -> (type_name, bytes, string_repr)
    defaults: HashMap<String, (String, Vec<u8>, String)>,
}

impl MapDefaultProvider {
    /// Create a new empty provider.
    pub fn new() -> Self {
        Self {
            defaults: HashMap::new(),
        }
    }

    /// Add a string default.
    pub fn with_string(mut self, field: &str, value: &str) -> Self {
        let bytes = value.as_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("String".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add an i32 default.
    pub fn with_i32(mut self, field: &str, value: i32) -> Self {
        let bytes = value.to_le_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("i32".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add an i64 default.
    pub fn with_i64(mut self, field: &str, value: i64) -> Self {
        let bytes = value.to_le_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("i64".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add a u32 default.
    pub fn with_u32(mut self, field: &str, value: u32) -> Self {
        let bytes = value.to_le_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("u32".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add a u64 default.
    pub fn with_u64(mut self, field: &str, value: u64) -> Self {
        let bytes = value.to_le_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("u64".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add an f32 default.
    pub fn with_f32(mut self, field: &str, value: f32) -> Self {
        let bytes = value.to_le_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("f32".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add an f64 default.
    pub fn with_f64(mut self, field: &str, value: f64) -> Self {
        let bytes = value.to_le_bytes().to_vec();
        self.defaults.insert(
            field.to_string(),
            ("f64".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add a bool default.
    pub fn with_bool(mut self, field: &str, value: bool) -> Self {
        let bytes = vec![if value { 1u8 } else { 0u8 }];
        self.defaults.insert(
            field.to_string(),
            ("bool".to_string(), bytes, value.to_string()),
        );
        self
    }

    /// Add raw bytes default.
    pub fn with_bytes(mut self, field: &str, type_name: &str, bytes: Vec<u8>, repr: &str) -> Self {
        self.defaults.insert(
            field.to_string(),
            (type_name.to_string(), bytes, repr.to_string()),
        );
        self
    }
}

impl DefaultProvider for MapDefaultProvider {
    fn default_bytes(&self, field: &str, _type_name: &str) -> Option<Vec<u8>> {
        self.defaults.get(field).map(|(_, bytes, _)| bytes.clone())
    }

    fn default_string(&self, field: &str, _type_name: &str) -> Option<String> {
        self.defaults.get(field).map(|(_, _, repr)| repr.clone())
    }

    fn has_default(&self, field: &str) -> bool {
        self.defaults.contains_key(field)
    }

    fn fields_with_defaults(&self) -> Vec<String> {
        self.defaults.keys().cloned().collect()
    }
}

/// Registry of default value providers.
///
/// Stores default providers for different schemas, enabling
/// runtime lookup of field defaults during migration.
pub struct DefaultRegistry {
    /// Providers keyed by schema name.
    providers: RwLock<HashMap<String, Arc<dyn DefaultProvider>>>,
}

impl DefaultRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a default provider for a schema.
    pub fn register(&self, schema: &str, provider: impl DefaultProvider + 'static) {
        let mut providers = self.providers.write();
        providers.insert(schema.to_string(), Arc::new(provider));
    }

    /// Register a shared default provider for a schema.
    pub fn register_shared(&self, schema: &str, provider: Arc<dyn DefaultProvider>) {
        let mut providers = self.providers.write();
        providers.insert(schema.to_string(), provider);
    }

    /// Get the default provider for a schema.
    pub fn get_provider(&self, schema: &str) -> Option<Arc<dyn DefaultProvider>> {
        let providers = self.providers.read();
        providers.get(schema).cloned()
    }

    /// Get a default value for a field in a schema.
    pub fn get_default(&self, schema: &str, field: &str, type_name: &str) -> Option<Vec<u8>> {
        self.get_provider(schema)
            .and_then(|p| p.default_bytes(field, type_name))
    }

    /// Get a default value as string representation.
    pub fn get_default_string(&self, schema: &str, field: &str, type_name: &str) -> Option<String> {
        self.get_provider(schema)
            .and_then(|p| p.default_string(field, type_name))
    }

    /// Check if a field has a default in a schema.
    pub fn has_default(&self, schema: &str, field: &str) -> bool {
        self.get_provider(schema)
            .map(|p| p.has_default(field))
            .unwrap_or(false)
    }

    /// Get all schemas with registered providers.
    pub fn schemas(&self) -> Vec<String> {
        let providers = self.providers.read();
        providers.keys().cloned().collect()
    }

    /// Unregister a default provider.
    pub fn unregister(&self, schema: &str) -> bool {
        let mut providers = self.providers.write();
        providers.remove(schema).is_some()
    }
}

impl Default for DefaultRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Field default specification for macro generation.
///
/// This struct captures default value information that can be
/// used by the `#[xerv::schema]` macro.
#[derive(Debug, Clone)]
pub struct FieldDefault {
    /// The name of the field.
    pub field: String,
    /// The type name of the field.
    pub type_name: String,
    /// Default value expression as a code string (used for code generation).
    pub expression: String,
    /// Serialized bytes of the default value (for runtime use).
    pub bytes: Option<Vec<u8>>,
}

impl FieldDefault {
    /// Create a new field default specification.
    pub fn new(
        field: impl Into<String>,
        type_name: impl Into<String>,
        expression: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            type_name: type_name.into(),
            expression: expression.into(),
            bytes: None,
        }
    }

    /// Set the serialized bytes.
    pub fn with_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.bytes = Some(bytes);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_provider_string_default() {
        let provider = MapDefaultProvider::new().with_string("currency", "USD");

        assert!(provider.has_default("currency"));
        assert!(!provider.has_default("other"));

        let bytes = provider.default_bytes("currency", "String").unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "USD");

        let repr = provider.default_string("currency", "String").unwrap();
        assert_eq!(repr, "USD");
    }

    #[test]
    fn map_provider_numeric_defaults() {
        let provider = MapDefaultProvider::new()
            .with_i32("count", 42)
            .with_f64("rate", 0.05)
            .with_bool("active", true);

        let count_bytes = provider.default_bytes("count", "i32").unwrap();
        assert_eq!(i32::from_le_bytes(count_bytes.try_into().unwrap()), 42);

        let rate_bytes = provider.default_bytes("rate", "f64").unwrap();
        assert_eq!(f64::from_le_bytes(rate_bytes.try_into().unwrap()), 0.05);

        let active_bytes = provider.default_bytes("active", "bool").unwrap();
        assert_eq!(active_bytes[0], 1);
    }

    #[test]
    fn registry_register_and_get() {
        let registry = DefaultRegistry::new();

        let provider = MapDefaultProvider::new()
            .with_string("currency", "USD")
            .with_f64("amount", 0.0);

        registry.register("OrderInput@v2", provider);

        assert!(registry.has_default("OrderInput@v2", "currency"));
        assert!(!registry.has_default("OrderInput@v2", "other"));
        assert!(!registry.has_default("OrderInput@v1", "currency"));

        let default = registry.get_default("OrderInput@v2", "currency", "String");
        assert!(default.is_some());

        let repr = registry.get_default_string("OrderInput@v2", "currency", "String");
        assert_eq!(repr, Some("USD".to_string()));
    }

    #[test]
    fn registry_schemas() {
        let registry = DefaultRegistry::new();

        registry.register("Schema1", MapDefaultProvider::new());
        registry.register("Schema2", MapDefaultProvider::new());

        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 2);
        assert!(schemas.contains(&"Schema1".to_string()));
        assert!(schemas.contains(&"Schema2".to_string()));
    }

    #[test]
    fn registry_unregister() {
        let registry = DefaultRegistry::new();

        registry.register("Schema1", MapDefaultProvider::new());
        assert!(registry.unregister("Schema1"));
        assert!(!registry.unregister("Schema1"));
        assert!(registry.schemas().is_empty());
    }

    #[test]
    fn fields_with_defaults() {
        let provider = MapDefaultProvider::new()
            .with_string("a", "value")
            .with_i32("b", 0)
            .with_bool("c", false);

        let fields = provider.fields_with_defaults();
        assert_eq!(fields.len(), 3);
        assert!(fields.contains(&"a".to_string()));
        assert!(fields.contains(&"b".to_string()));
        assert!(fields.contains(&"c".to_string()));
    }
}
