//! Trigger definition from YAML.

use crate::traits::{TriggerConfig, TriggerType};
use serde::{Deserialize, Serialize};

/// A trigger definition from YAML.
///
/// # Example
///
/// ```yaml
/// triggers:
///   - id: api_webhook
///     type: webhook
///     params:
///       host: "0.0.0.0"
///       port: 8080
///       path: "/api/orders"
///       method: "POST"
///
///   - id: daily_sync
///     type: cron
///     params:
///       schedule: "0 0 * * * *"  # Every hour
///
///   - id: file_watcher
///     type: filesystem
///     params:
///       path: "/data/uploads"
///       recursive: true
///       events: ["create", "modify"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDefinition {
    /// Unique identifier for this trigger.
    pub id: String,

    /// Trigger type (webhook, cron, filesystem, queue, memory, manual).
    #[serde(rename = "type")]
    pub trigger_type: String,

    /// Type-specific parameters.
    #[serde(default)]
    pub params: serde_yaml::Value,

    /// Whether the trigger is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl TriggerDefinition {
    /// Create a new trigger definition.
    pub fn new(id: impl Into<String>, trigger_type: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            trigger_type: trigger_type.into(),
            params: serde_yaml::Value::Null,
            enabled: true,
            description: None,
        }
    }

    /// Set parameters.
    pub fn with_params(mut self, params: serde_yaml::Value) -> Self {
        self.params = params;
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Disable the trigger.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Parse the trigger type string.
    pub fn parsed_type(&self) -> Option<TriggerType> {
        TriggerType::parse(&self.trigger_type)
    }

    /// Convert to TriggerConfig for use with trigger factories.
    pub fn to_trigger_config(&self) -> Option<TriggerConfig> {
        let trigger_type = self.parsed_type()?;
        Some(TriggerConfig {
            id: self.id.clone(),
            trigger_type,
            params: self.params.clone(),
        })
    }

    /// Get a string parameter.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }

    /// Get an integer parameter.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.params.get(key).and_then(|v| v.as_i64())
    }

    /// Get a boolean parameter.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.params.get(key).and_then(|v| v.as_bool())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_webhook_trigger() {
        let yaml = r#"
id: api_webhook
type: webhook
params:
  host: "0.0.0.0"
  port: 8080
  path: "/orders"
"#;
        let trigger: TriggerDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(trigger.id, "api_webhook");
        assert_eq!(trigger.trigger_type, "webhook");
        assert_eq!(trigger.parsed_type(), Some(TriggerType::Webhook));
        assert_eq!(trigger.get_string("host"), Some("0.0.0.0"));
        assert_eq!(trigger.get_i64("port"), Some(8080));
        assert!(trigger.enabled);
    }

    #[test]
    fn deserialize_cron_trigger() {
        let yaml = r#"
id: hourly_sync
type: cron
params:
  schedule: "0 0 * * * *"
description: "Runs every hour"
"#;
        let trigger: TriggerDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(trigger.id, "hourly_sync");
        assert_eq!(trigger.trigger_type, "cron");
        assert_eq!(trigger.parsed_type(), Some(TriggerType::Cron));
        assert_eq!(trigger.get_string("schedule"), Some("0 0 * * * *"));
        assert_eq!(trigger.description, Some("Runs every hour".to_string()));
    }

    #[test]
    fn to_trigger_config() {
        let def = TriggerDefinition::new("test", "webhook")
            .with_params(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

        let config = def.to_trigger_config().unwrap();
        assert_eq!(config.id, "test");
        assert_eq!(config.trigger_type, TriggerType::Webhook);
    }

    #[test]
    fn invalid_trigger_type() {
        let def = TriggerDefinition::new("test", "invalid_type");
        assert!(def.parsed_type().is_none());
        assert!(def.to_trigger_config().is_none());
    }

    #[test]
    fn disabled_trigger() {
        let yaml = r#"
id: disabled_webhook
type: webhook
enabled: false
"#;
        let trigger: TriggerDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(!trigger.enabled);
    }
}
