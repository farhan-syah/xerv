//! Flow validation logic.

use std::collections::HashSet;

use super::ValidationResult;
use super::error::{ValidationError, ValidationErrorKind};
use super::limits::ValidationLimits;
use crate::flow::{FlowDefinition, NodeDefinition, TriggerDefinition};

/// Validator for flow definitions.
pub struct FlowValidator {
    errors: Vec<ValidationError>,
    limits: ValidationLimits,
}

impl FlowValidator {
    /// Create a new validator with default limits.
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            limits: ValidationLimits::default(),
        }
    }

    /// Create a validator with custom limits.
    pub fn with_limits(limits: ValidationLimits) -> Self {
        Self {
            errors: Vec::new(),
            limits,
        }
    }

    /// Validate a flow definition.
    pub fn validate(mut self, flow: &FlowDefinition) -> ValidationResult {
        // Check structural limits first (DoS protection)
        self.validate_limits(flow);

        // Then validate semantics
        self.validate_metadata(flow);
        self.validate_triggers(flow);
        self.validate_nodes(flow);
        self.validate_edges(flow);
        self.validate_references(flow);

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }

    fn validate_limits(&mut self, flow: &FlowDefinition) {
        // Check node count
        if flow.nodes.len() > self.limits.max_node_count {
            self.add_error(ValidationError::new(
                ValidationErrorKind::LimitExceeded,
                "nodes",
                format!(
                    "node count ({}) exceeds maximum allowed ({})",
                    flow.nodes.len(),
                    self.limits.max_node_count
                ),
            ));
        }

        // Check edge count
        if flow.edges.len() > self.limits.max_edge_count {
            self.add_error(ValidationError::new(
                ValidationErrorKind::LimitExceeded,
                "edges",
                format!(
                    "edge count ({}) exceeds maximum allowed ({})",
                    flow.edges.len(),
                    self.limits.max_edge_count
                ),
            ));
        }

        // Check trigger count
        if flow.triggers.len() > self.limits.max_trigger_count {
            self.add_error(ValidationError::new(
                ValidationErrorKind::LimitExceeded,
                "triggers",
                format!(
                    "trigger count ({}) exceeds maximum allowed ({})",
                    flow.triggers.len(),
                    self.limits.max_trigger_count
                ),
            ));
        }
    }

    fn add_error(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    fn validate_metadata(&mut self, flow: &FlowDefinition) {
        if flow.name.is_empty() {
            self.add_error(ValidationError::missing_field("flow", "name"));
        }

        if let Some(ref version) = flow.version {
            if version.is_empty() {
                self.add_error(ValidationError::invalid_value(
                    "flow.version",
                    "version cannot be empty string",
                ));
            }
        }
    }

    fn validate_triggers(&mut self, flow: &FlowDefinition) {
        let mut seen_ids = HashSet::new();

        for (idx, trigger) in flow.triggers.iter().enumerate() {
            let location = format!("triggers[{}]", idx);

            // Check for duplicate IDs
            if !seen_ids.insert(&trigger.id) {
                self.add_error(ValidationError::duplicate_id(&location, &trigger.id));
            }

            // Validate trigger ID
            if trigger.id.is_empty() {
                self.add_error(ValidationError::missing_field(&location, "id"));
            }

            // Validate trigger type
            if trigger.parsed_type().is_none() {
                self.add_error(ValidationError::new(
                    ValidationErrorKind::InvalidTriggerType,
                    &location,
                    format!("unknown trigger type '{}'", trigger.trigger_type),
                ));
            }

            // Type-specific validation
            self.validate_trigger_params(trigger, &location);
        }
    }

    fn validate_trigger_params(&mut self, trigger: &TriggerDefinition, location: &str) {
        match trigger.trigger_type.as_str() {
            "webhook" | "trigger::webhook" => {
                // Webhook triggers should have port
                if let Some(port) = trigger.get_i64("port") {
                    if !(1..=65535).contains(&port) {
                        self.add_error(ValidationError::invalid_value(
                            format!("{}.params.port", location),
                            format!("port must be between 1 and 65535, got {}", port),
                        ));
                    }
                }
            }
            "cron" | "trigger::cron" => {
                // Cron triggers must have schedule
                if trigger.get_string("schedule").is_none() {
                    self.add_error(ValidationError::missing_field(
                        format!("{}.params", location),
                        "schedule",
                    ));
                }
            }
            "filesystem" | "trigger::filesystem" => {
                // Filesystem triggers must have path
                if trigger.get_string("path").is_none() {
                    self.add_error(ValidationError::missing_field(
                        format!("{}.params", location),
                        "path",
                    ));
                }
            }
            _ => {}
        }
    }

    fn validate_nodes(&mut self, flow: &FlowDefinition) {
        let mut seen_ids = HashSet::new();

        for (node_id, node) in &flow.nodes {
            let location = format!("nodes.{}", node_id);

            // Check for duplicate IDs (shouldn't happen with HashMap, but check trigger IDs too)
            if !seen_ids.insert(node_id) {
                self.add_error(ValidationError::duplicate_id(&location, node_id));
            }

            // Check node ID doesn't conflict with trigger IDs
            for trigger in &flow.triggers {
                if &trigger.id == node_id {
                    self.add_error(ValidationError::new(
                        ValidationErrorKind::DuplicateId,
                        &location,
                        format!("node ID conflicts with trigger ID '{}'", node_id),
                    ));
                }
            }

            // Validate node type
            if node.node_type.is_empty() {
                self.add_error(ValidationError::missing_field(&location, "type"));
            }

            // Type-specific validation
            self.validate_node_config(node, node_id, &location);
        }
    }

    fn validate_node_config(&mut self, node: &NodeDefinition, _node_id: &str, location: &str) {
        match node.node_type.as_str() {
            "std::switch" => {
                // Switch nodes should have condition
                if node.get_nested(&["condition"]).is_none()
                    && node.get_string("expression").is_none()
                {
                    self.add_error(ValidationError::missing_field(
                        format!("{}.config", location),
                        "condition or expression",
                    ));
                }
            }
            "std::loop" => {
                // Loop nodes should have max_iterations or condition
                if node.get_i64("max_iterations").is_none()
                    && node.get_nested(&["condition"]).is_none()
                {
                    self.add_error(ValidationError::missing_field(
                        format!("{}.config", location),
                        "max_iterations or condition",
                    ));
                }
            }
            "std::merge" => {
                // Merge nodes can optionally specify input_count or strategy
            }
            "std::aggregate" => {
                // Aggregate nodes should have operation
                if node.get_string("operation").is_none() {
                    self.add_error(ValidationError::missing_field(
                        format!("{}.config", location),
                        "operation",
                    ));
                }
            }
            _ => {}
        }
    }

    fn validate_edges(&mut self, flow: &FlowDefinition) {
        for (idx, edge) in flow.edges.iter().enumerate() {
            let location = format!("edges[{}]", idx);

            // Validate from
            if edge.from.is_empty() {
                self.add_error(ValidationError::missing_field(&location, "from"));
            }

            // Validate to
            if edge.to.is_empty() {
                self.add_error(ValidationError::missing_field(&location, "to"));
            }

            // Validate condition syntax if present
            if let Some(ref condition) = edge.condition {
                self.validate_selector_syntax(condition, &format!("{}.condition", location));
            }
        }
    }

    fn validate_references(&mut self, flow: &FlowDefinition) {
        // Collect all valid node IDs
        let mut valid_ids: HashSet<&str> = flow.nodes.keys().map(|s| s.as_str()).collect();

        // Triggers are also valid sources
        for trigger in &flow.triggers {
            valid_ids.insert(&trigger.id);
        }

        // Check edge references
        for (idx, edge) in flow.edges.iter().enumerate() {
            let location = format!("edges[{}]", idx);

            let from_node = edge.from_node();
            if !valid_ids.contains(from_node) {
                self.add_error(ValidationError::invalid_reference(
                    format!("{}.from", location),
                    from_node,
                ));
            }

            let to_node = edge.to_node();
            if !valid_ids.contains(to_node) {
                self.add_error(ValidationError::invalid_reference(
                    format!("{}.to", location),
                    to_node,
                ));
            }
        }
    }

    fn validate_selector_syntax(&mut self, selector: &str, location: &str) {
        // Basic selector syntax validation
        // Selectors look like: ${node.field} or ${node.field.subfield}
        let mut in_selector = false;
        let mut brace_depth = 0;

        for c in selector.chars() {
            match c {
                '$' => {
                    // Could be start of selector
                }
                '{' if in_selector || selector.contains("${") => {
                    brace_depth += 1;
                    in_selector = true;
                }
                '}' if in_selector => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        in_selector = false;
                    }
                }
                _ => {}
            }
        }

        if brace_depth != 0 {
            self.add_error(ValidationError::new(
                ValidationErrorKind::InvalidSelector,
                location,
                "unbalanced braces in selector",
            ));
        }
    }
}

impl Default for FlowValidator {
    fn default() -> Self {
        Self::new()
    }
}
