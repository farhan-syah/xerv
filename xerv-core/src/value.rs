//! Dynamic value type for condition evaluation.
//!
//! Provides a flexible value type for field access and comparison
//! in flow control nodes (switch, loop, etc.).

use crate::error::{Result, XervError};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Dynamic value for field access and condition evaluation.
///
/// Wraps serde_json::Value to provide type-safe field extraction
/// and comparison operations used by flow control nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Value(pub JsonValue);

impl Value {
    /// Create a null value.
    pub fn null() -> Self {
        Self(JsonValue::Null)
    }

    /// Create a boolean value.
    pub fn bool(v: bool) -> Self {
        Self(JsonValue::Bool(v))
    }

    /// Create an integer value.
    pub fn int(v: i64) -> Self {
        Self(JsonValue::Number(v.into()))
    }

    /// Create a floating-point value.
    pub fn float(v: f64) -> Self {
        Self(serde_json::Number::from_f64(v).map_or(JsonValue::Null, JsonValue::Number))
    }

    /// Create a string value.
    pub fn string(v: impl Into<String>) -> Self {
        Self(JsonValue::String(v.into()))
    }

    /// Create a value from JSON bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Ok(Self::null());
        }
        serde_json::from_slice(bytes)
            .map(Self)
            .map_err(|e| XervError::Serialization(format!("Failed to parse value: {}", e)))
    }

    /// Serialize to JSON bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.0)
            .map_err(|e| XervError::Serialization(format!("Failed to serialize value: {}", e)))
    }

    /// Check if the value is null.
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    /// Get a field by path (dot notation or JSONPath-like).
    ///
    /// Supports:
    /// - Simple field access: "field"
    /// - Dot notation: "parent.child.value"
    /// - JSONPath prefix: "$.parent.child" -> "parent.child"
    ///
    /// Returns None if the field doesn't exist.
    pub fn get_field(&self, path: &str) -> Option<Value> {
        // Strip JSONPath prefix if present
        let path = path.strip_prefix("$.").unwrap_or(path);

        let mut current = &self.0;
        for part in path.split('.') {
            // Handle array index notation: field[0]
            if let Some((field, idx_str)) = part.split_once('[') {
                current = current.get(field)?;
                let idx_str = idx_str.strip_suffix(']')?;
                let idx: usize = idx_str.parse().ok()?;
                current = current.get(idx)?;
            } else {
                current = current.get(part)?;
            }
        }
        Some(Value(current.clone()))
    }

    /// Get a field as a string.
    pub fn get_string(&self, path: &str) -> Option<String> {
        self.get_field(path).and_then(|v| v.as_string())
    }

    /// Get a field as an f64.
    pub fn get_f64(&self, path: &str) -> Option<f64> {
        self.get_field(path).and_then(|v| v.as_f64())
    }

    /// Get a field as a bool.
    pub fn get_bool(&self, path: &str) -> Option<bool> {
        self.get_field(path).and_then(|v| v.as_bool())
    }

    /// Convert to string if possible.
    pub fn as_string(&self) -> Option<String> {
        match &self.0 {
            JsonValue::String(s) => Some(s.clone()),
            JsonValue::Number(n) => Some(n.to_string()),
            JsonValue::Bool(b) => Some(b.to_string()),
            JsonValue::Null => None,
            _ => Some(self.0.to_string()),
        }
    }

    /// Convert to f64 if possible.
    pub fn as_f64(&self) -> Option<f64> {
        match &self.0 {
            JsonValue::Number(n) => n.as_f64(),
            JsonValue::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Convert to bool if possible.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.0 {
            JsonValue::Bool(b) => Some(*b),
            JsonValue::String(s) => match s.to_lowercase().as_str() {
                "true" | "1" | "yes" => Some(true),
                "false" | "0" | "no" => Some(false),
                _ => None,
            },
            JsonValue::Number(n) => Some(n.as_f64().is_some_and(|v| v != 0.0)),
            JsonValue::Null => Some(false),
            _ => None,
        }
    }

    /// Check equality with a string value.
    pub fn equals_str(&self, other: &str) -> bool {
        self.as_string().is_some_and(|s| s == other)
    }

    /// Check if a field equals a value (string comparison).
    pub fn field_equals(&self, path: &str, value: &str) -> bool {
        self.get_field(path).is_some_and(|v| v.equals_str(value))
    }

    /// Check if a field is greater than a threshold (numeric comparison).
    pub fn field_greater_than(&self, path: &str, threshold: f64) -> bool {
        self.get_f64(path).is_some_and(|v| v > threshold)
    }

    /// Check if a field is less than a threshold (numeric comparison).
    pub fn field_less_than(&self, path: &str, threshold: f64) -> bool {
        self.get_f64(path).is_some_and(|v| v < threshold)
    }

    /// Check if a field matches a regex pattern.
    pub fn field_matches(&self, path: &str, pattern: &str) -> bool {
        let Some(field_value) = self.get_string(path) else {
            return false;
        };
        regex::Regex::new(pattern)
            .map(|re| re.is_match(&field_value))
            .unwrap_or(false)
    }

    /// Check if a boolean field is true.
    pub fn field_is_true(&self, path: &str) -> bool {
        self.get_bool(path).unwrap_or(false)
    }

    /// Check if a boolean field is false.
    pub fn field_is_false(&self, path: &str) -> bool {
        self.get_bool(path).is_some_and(|b| !b)
    }

    /// Access the inner serde_json::Value.
    pub fn inner(&self) -> &JsonValue {
        &self.0
    }

    /// Convert into the inner serde_json::Value.
    pub fn into_inner(self) -> JsonValue {
        self.0
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::null()
    }
}

impl From<JsonValue> for Value {
    fn from(v: JsonValue) -> Self {
        Self(v)
    }
}

impl From<Value> for JsonValue {
    fn from(v: Value) -> Self {
        v.0
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::string(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::string(s)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::int(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::float(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::bool(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn value_from_bytes() {
        let bytes = br#"{"name": "test", "score": 0.95}"#;
        let value = Value::from_bytes(bytes).unwrap();

        assert_eq!(value.get_string("name"), Some("test".to_string()));
        assert_eq!(value.get_f64("score"), Some(0.95));
    }

    #[test]
    fn value_nested_field_access() {
        let value = Value(json!({
            "result": {
                "status": "success",
                "data": {
                    "count": 42
                }
            }
        }));

        assert_eq!(
            value.get_string("result.status"),
            Some("success".to_string())
        );
        assert_eq!(value.get_f64("result.data.count"), Some(42.0));
    }

    #[test]
    fn value_jsonpath_prefix() {
        let value = Value(json!({"score": 0.9}));

        // Both should work
        assert_eq!(value.get_f64("score"), Some(0.9));
        assert_eq!(value.get_f64("$.score"), Some(0.9));
    }

    #[test]
    fn value_array_access() {
        let value = Value(json!({
            "items": [
                {"name": "first"},
                {"name": "second"}
            ]
        }));

        assert_eq!(value.get_string("items[0].name"), Some("first".to_string()));
        assert_eq!(
            value.get_string("items[1].name"),
            Some("second".to_string())
        );
    }

    #[test]
    fn field_equals() {
        let value = Value(json!({"status": "active"}));
        assert!(value.field_equals("status", "active"));
        assert!(!value.field_equals("status", "inactive"));
    }

    #[test]
    fn field_greater_than() {
        let value = Value(json!({"score": 0.85}));
        assert!(value.field_greater_than("score", 0.8));
        assert!(!value.field_greater_than("score", 0.9));
    }

    #[test]
    fn field_less_than() {
        let value = Value(json!({"temperature": 25.5}));
        assert!(value.field_less_than("temperature", 30.0));
        assert!(!value.field_less_than("temperature", 20.0));
    }

    #[test]
    fn field_matches() {
        let value = Value(json!({"email": "user@example.com"}));
        assert!(value.field_matches("email", r"^[\w.+-]+@[\w.-]+\.\w+$"));
        assert!(!value.field_matches("email", r"^invalid"));
    }

    #[test]
    fn field_bool_checks() {
        let value = Value(json!({"success": true, "failed": false}));
        assert!(value.field_is_true("success"));
        assert!(!value.field_is_true("failed"));
        assert!(value.field_is_false("failed"));
        assert!(!value.field_is_false("success"));
    }

    #[test]
    fn empty_bytes_returns_null() {
        let value = Value::from_bytes(&[]).unwrap();
        assert!(value.is_null());
    }

    #[test]
    fn missing_field_returns_none() {
        let value = Value(json!({"a": 1}));
        assert!(value.get_field("missing").is_none());
        assert!(value.get_f64("missing").is_none());
    }
}
