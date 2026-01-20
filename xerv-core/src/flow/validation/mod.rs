//! Flow validation.
//!
//! This module provides validation for flow definitions, including:
//! - Structural validation (node count, edge count, nesting depth)
//! - Semantic validation (references, types, required fields)
//! - DoS protection (size limits, depth limits)

mod error;
mod limits;
mod validator;

pub use error::{ValidationError, ValidationErrorKind};
pub use limits::ValidationLimits;
pub use validator::FlowValidator;

/// Result of flow validation.
pub type ValidationResult = Result<(), Vec<ValidationError>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{EdgeDefinition, FlowDefinition, NodeDefinition, TriggerDefinition};
    use std::collections::HashMap;

    fn minimal_flow() -> FlowDefinition {
        FlowDefinition {
            name: "test".to_string(),
            version: Some("1.0".to_string()),
            description: None,
            triggers: vec![TriggerDefinition::new("webhook", "webhook")],
            nodes: HashMap::new(),
            edges: vec![],
            settings: Default::default(),
        }
    }

    #[test]
    fn validate_minimal_flow() {
        let flow = minimal_flow();
        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_missing_name() {
        let mut flow = minimal_flow();
        flow.name = String::new();

        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::MissingField && e.location == "flow")
        );
    }

    #[test]
    fn validate_duplicate_trigger_ids() {
        let mut flow = minimal_flow();
        flow.triggers = vec![
            TriggerDefinition::new("dup_id", "webhook"),
            TriggerDefinition::new("dup_id", "cron"),
        ];

        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::DuplicateId)
        );
    }

    #[test]
    fn validate_invalid_trigger_type() {
        let mut flow = minimal_flow();
        flow.triggers = vec![TriggerDefinition::new("test", "invalid_type")];

        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::InvalidTriggerType)
        );
    }

    #[test]
    fn validate_invalid_edge_reference() {
        let mut flow = minimal_flow();
        flow.edges = vec![EdgeDefinition::new("nonexistent", "also_nonexistent")];

        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::InvalidReference)
        );
    }

    #[test]
    fn validate_valid_edge_reference() {
        let mut flow = minimal_flow();
        flow.nodes
            .insert("processor".to_string(), NodeDefinition::new("std::log"));
        flow.edges = vec![EdgeDefinition::new("webhook", "processor")];

        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_cron_requires_schedule() {
        let mut flow = minimal_flow();
        flow.triggers = vec![TriggerDefinition::new("cron_trigger", "cron")];

        let result = FlowValidator::new().validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::MissingField
                    && e.message.contains("schedule"))
        );
    }

    // ========== Validation Limits Tests ==========

    #[test]
    fn validate_content_size_limit() {
        let limits = ValidationLimits::default().with_max_file_size(100);

        // Small content should pass
        let small_content = "name: test\n";
        assert!(limits.validate_content_size(small_content).is_ok());

        // Large content should fail
        let large_content = "a".repeat(200);
        let result = limits.validate_content_size(&large_content);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::LimitExceeded);
        assert!(error.message.contains("200 bytes"));
    }

    #[test]
    fn validate_nesting_depth_limit() {
        let limits = ValidationLimits::default().with_max_nesting_depth(3);

        // Shallow nesting should pass
        let shallow: serde_yaml::Value = serde_yaml::from_str("a: {b: 1}").unwrap();
        assert!(limits.validate_nesting_depth(&shallow).is_ok());

        // Deep nesting should fail
        let deep: serde_yaml::Value = serde_yaml::from_str("a: {b: {c: {d: {e: 1}}}}").unwrap();
        let result = limits.validate_nesting_depth(&deep);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind, ValidationErrorKind::LimitExceeded);
        assert!(error.message.contains("nesting depth"));
    }

    #[test]
    fn validate_node_count_limit() {
        let limits = ValidationLimits::default().with_max_node_count(2);
        let mut flow = minimal_flow();

        // Add 3 nodes (exceeds limit of 2)
        flow.nodes
            .insert("node1".to_string(), NodeDefinition::new("std::log"));
        flow.nodes
            .insert("node2".to_string(), NodeDefinition::new("std::log"));
        flow.nodes
            .insert("node3".to_string(), NodeDefinition::new("std::log"));

        let result = FlowValidator::with_limits(limits).validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::LimitExceeded
                    && e.message.contains("node count"))
        );
    }

    #[test]
    fn validate_edge_count_limit() {
        let mut limits = ValidationLimits::default();
        limits.max_edge_count = 1;

        let mut flow = minimal_flow();
        flow.nodes
            .insert("n1".to_string(), NodeDefinition::new("std::log"));
        flow.nodes
            .insert("n2".to_string(), NodeDefinition::new("std::log"));
        flow.edges = vec![
            EdgeDefinition::new("webhook", "n1"),
            EdgeDefinition::new("n1", "n2"),
        ];

        let result = FlowValidator::with_limits(limits).validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::LimitExceeded
                    && e.message.contains("edge count"))
        );
    }

    #[test]
    fn validate_trigger_count_limit() {
        let mut limits = ValidationLimits::default();
        limits.max_trigger_count = 1;

        let mut flow = minimal_flow();
        flow.triggers = vec![
            TriggerDefinition::new("t1", "webhook"),
            TriggerDefinition::new("t2", "webhook"),
        ];

        let result = FlowValidator::with_limits(limits).validate(&flow);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::LimitExceeded
                    && e.message.contains("trigger count"))
        );
    }

    #[test]
    fn measure_depth_flat() {
        let value: serde_yaml::Value = serde_yaml::from_str("a: 1\nb: 2").unwrap();
        assert_eq!(ValidationLimits::measure_depth(&value), 2);
    }

    #[test]
    fn measure_depth_nested_mapping() {
        let value: serde_yaml::Value = serde_yaml::from_str("a: {b: {c: 1}}").unwrap();
        assert_eq!(ValidationLimits::measure_depth(&value), 4);
    }

    #[test]
    fn measure_depth_sequence() {
        let value: serde_yaml::Value = serde_yaml::from_str("[1, [2, [3]]]").unwrap();
        assert_eq!(ValidationLimits::measure_depth(&value), 4);
    }
}
