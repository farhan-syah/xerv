//! Switch node (conditional routing).
//!
//! Routes data to different output ports based on a condition expression.
//! Similar to a conditional branch in programming.

use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// A condition that determines routing.
#[derive(Debug, Clone, Default)]
pub enum SwitchCondition {
    /// Always true (for default routing).
    #[default]
    Always,
    /// Check if a field equals a value.
    FieldEquals { field: String, value: String },
    /// Check if a field matches a regex pattern.
    FieldMatches { field: String, pattern: String },
    /// Check if a field is greater than a threshold.
    FieldGreaterThan { field: String, threshold: f64 },
    /// Check if a field is less than a threshold.
    FieldLessThan { field: String, threshold: f64 },
    /// Custom expression (e.g., "${input.score} > 0.8").
    Expression(String),
}

/// Switch node - conditional routing.
///
/// Routes incoming data to one of two output ports ("true" or "false")
/// based on evaluating a condition.
///
/// # Ports
/// - Input: "in" - The data to route
/// - Output: "true" - Activated when condition is true
/// - Output: "false" - Activated when condition is false
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   check_fraud:
///     type: std::switch
///     config:
///       condition:
///         type: field_greater_than
///         field: $.score
///         threshold: 0.8
///     inputs:
///       - from: fraud_model.out -> in
///     outputs:
///       true: -> high_risk.in
///       false: -> low_risk.in
/// ```
#[derive(Debug)]
pub struct SwitchNode {
    /// The condition to evaluate.
    condition: SwitchCondition,
}

impl SwitchNode {
    /// Create a switch node with the given condition.
    pub fn new(condition: SwitchCondition) -> Self {
        Self { condition }
    }

    /// Create a switch that routes based on field equality.
    pub fn field_equals(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            condition: SwitchCondition::FieldEquals {
                field: field.into(),
                value: value.into(),
            },
        }
    }

    /// Create a switch that routes based on a threshold.
    pub fn threshold(field: impl Into<String>, threshold: f64) -> Self {
        Self {
            condition: SwitchCondition::FieldGreaterThan {
                field: field.into(),
                threshold,
            },
        }
    }

    /// Create a switch that routes based on a custom expression.
    pub fn expression(expr: impl Into<String>) -> Self {
        Self {
            condition: SwitchCondition::Expression(expr.into()),
        }
    }

    /// Evaluate the condition against the input value.
    ///
    /// Returns `true` if the condition is satisfied, `false` otherwise.
    /// If the input is null or the field doesn't exist, returns `false`
    /// (except for `Always` which always returns `true`).
    fn evaluate(&self, value: &Value) -> bool {
        match &self.condition {
            SwitchCondition::Always => true,

            SwitchCondition::FieldEquals {
                field,
                value: expected,
            } => {
                let result = value.field_equals(field, expected);
                tracing::debug!(
                    field = %field,
                    expected = %expected,
                    result = result,
                    "Evaluated field_equals condition"
                );
                result
            }

            SwitchCondition::FieldMatches { field, pattern } => {
                let result = value.field_matches(field, pattern);
                tracing::debug!(
                    field = %field,
                    pattern = %pattern,
                    result = result,
                    "Evaluated field_matches condition"
                );
                result
            }

            SwitchCondition::FieldGreaterThan { field, threshold } => {
                let result = value.field_greater_than(field, *threshold);
                tracing::debug!(
                    field = %field,
                    threshold = %threshold,
                    result = result,
                    "Evaluated field_greater_than condition"
                );
                result
            }

            SwitchCondition::FieldLessThan { field, threshold } => {
                let result = value.field_less_than(field, *threshold);
                tracing::debug!(
                    field = %field,
                    threshold = %threshold,
                    result = result,
                    "Evaluated field_less_than condition"
                );
                result
            }

            SwitchCondition::Expression(expr) => {
                // Expression evaluation is a simplified subset:
                // - "${field} == value" -> field_equals
                // - "${field} > value" -> field_greater_than
                // - "${field} < value" -> field_less_than
                // - "${field}" -> field_is_true (boolean check)
                let result = self.evaluate_expression(expr, value);
                tracing::debug!(
                    expr = %expr,
                    result = result,
                    "Evaluated expression condition"
                );
                result
            }
        }
    }

    /// Evaluate a simple expression against the input value.
    ///
    /// Supports basic patterns:
    /// - `${field}` -> checks if field is truthy
    /// - `${field} == "value"` -> string equality
    /// - `${field} > number` -> numeric greater than
    /// - `${field} < number` -> numeric less than
    /// - `${field} >= number` -> numeric greater than or equal
    /// - `${field} <= number` -> numeric less than or equal
    fn evaluate_expression(&self, expr: &str, value: &Value) -> bool {
        let expr = expr.trim();

        // Try to parse comparison expressions
        if let Some((field, op, rhs)) = self.parse_comparison(expr) {
            match op {
                "==" | "=" => {
                    // String equality (strip quotes from rhs)
                    let rhs = rhs.trim_matches('"').trim_matches('\'');
                    value.field_equals(&field, rhs)
                }
                "!=" => {
                    let rhs = rhs.trim_matches('"').trim_matches('\'');
                    !value.field_equals(&field, rhs)
                }
                ">" => {
                    if let Ok(threshold) = rhs.parse::<f64>() {
                        value.field_greater_than(&field, threshold)
                    } else {
                        false
                    }
                }
                "<" => {
                    if let Ok(threshold) = rhs.parse::<f64>() {
                        value.field_less_than(&field, threshold)
                    } else {
                        false
                    }
                }
                ">=" => {
                    if let Ok(threshold) = rhs.parse::<f64>() {
                        value.get_f64(&field).is_some_and(|v| v >= threshold)
                    } else {
                        false
                    }
                }
                "<=" => {
                    if let Ok(threshold) = rhs.parse::<f64>() {
                        value.get_f64(&field).is_some_and(|v| v <= threshold)
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else if let Some(field) = self.parse_field_ref(expr) {
            // Just a field reference - check if truthy
            value.field_is_true(&field)
        } else {
            // Unrecognized expression format
            tracing::warn!(expr = %expr, "Unrecognized expression format");
            false
        }
    }

    /// Parse a comparison expression like "${field} > 0.5"
    fn parse_comparison<'a>(&self, expr: &'a str) -> Option<(String, &'a str, &'a str)> {
        // Operators in order of specificity (>= before >)
        let operators = [">=", "<=", "==", "!=", ">", "<", "="];

        for op in operators {
            if let Some(pos) = expr.find(op) {
                let lhs = expr[..pos].trim();
                let rhs = expr[pos + op.len()..].trim();

                // Extract field from ${field} or $.field syntax
                if let Some(field) = self.parse_field_ref(lhs) {
                    return Some((field, op, rhs));
                }
            }
        }
        None
    }

    /// Parse a field reference like "${field}" or "$.field"
    fn parse_field_ref(&self, s: &str) -> Option<String> {
        let s = s.trim();

        // ${field.path} format
        if s.starts_with("${") && s.ends_with('}') {
            return Some(s[2..s.len() - 1].to_string());
        }

        // $.field.path format
        if let Some(stripped) = s.strip_prefix("$.") {
            return Some(stripped.to_string());
        }

        // Plain field name (if it looks like an identifier)
        if !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Some(s.to_string());
        }

        None
    }
}

impl Node for SwitchNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "switch")
            .with_description("Conditional routing based on expression")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("true", PortDirection::Output, "Any"),
                Port::named("false", PortDirection::Output, "Any"),
                Port::error(),
            ])
    }

    fn execute<'a>(&'a self, ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").copied().unwrap_or_else(RelPtr::null);

            // Read and parse input data from arena
            let value = if input.is_null() {
                Value::null()
            } else {
                match ctx.read_bytes(input) {
                    Ok(bytes) => Value::from_bytes(&bytes).unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "Failed to parse input as JSON, using null");
                        Value::null()
                    }),
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to read input from arena, using null");
                        Value::null()
                    }
                }
            };

            let result = self.evaluate(&value);

            tracing::debug!(condition_result = result, "Switch evaluated condition");

            if result {
                Ok(NodeOutput::on_true(input))
            } else {
                Ok(NodeOutput::on_false(input))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn switch_node_info() {
        let node = SwitchNode::new(SwitchCondition::Always);
        let info = node.info();

        assert_eq!(info.name, "std::switch");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.inputs[0].name, "in");
        assert_eq!(info.outputs.len(), 3);
        assert_eq!(info.outputs[0].name, "true");
        assert_eq!(info.outputs[1].name, "false");
    }

    #[test]
    fn switch_condition_always() {
        let node = SwitchNode::new(SwitchCondition::Always);
        let value = Value::null();
        assert!(node.evaluate(&value));
    }

    #[test]
    fn switch_threshold_creation() {
        let node = SwitchNode::threshold("score", 0.8);
        assert!(matches!(
            node.condition,
            SwitchCondition::FieldGreaterThan { threshold, .. } if threshold == 0.8
        ));
    }

    #[test]
    fn switch_field_equals() {
        let node = SwitchNode::field_equals("status", "active");
        let value = Value(json!({"status": "active"}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"status": "inactive"}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_field_greater_than() {
        let node = SwitchNode::threshold("score", 0.8);

        let value = Value(json!({"score": 0.9}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"score": 0.7}));
        assert!(!node.evaluate(&value));

        let value = Value(json!({"score": 0.8}));
        assert!(!node.evaluate(&value)); // not strictly greater than
    }

    #[test]
    fn switch_field_less_than() {
        let node = SwitchNode::new(SwitchCondition::FieldLessThan {
            field: "temperature".to_string(),
            threshold: 30.0,
        });

        let value = Value(json!({"temperature": 25.0}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"temperature": 35.0}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_field_matches() {
        let node = SwitchNode::new(SwitchCondition::FieldMatches {
            field: "email".to_string(),
            pattern: r"^[\w.+-]+@[\w.-]+\.\w+$".to_string(),
        });

        let value = Value(json!({"email": "test@example.com"}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"email": "invalid-email"}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_expression_comparison() {
        let node = SwitchNode::expression("${score} > 0.5");

        let value = Value(json!({"score": 0.7}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"score": 0.3}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_expression_equality() {
        let node = SwitchNode::expression("${status} == \"success\"");

        let value = Value(json!({"status": "success"}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"status": "failed"}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_expression_boolean_field() {
        let node = SwitchNode::expression("${is_valid}");

        let value = Value(json!({"is_valid": true}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"is_valid": false}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_nested_field_access() {
        let node = SwitchNode::field_equals("result.status", "ok");

        let value = Value(json!({"result": {"status": "ok"}}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"result": {"status": "error"}}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_missing_field_returns_false() {
        let node = SwitchNode::field_equals("nonexistent", "value");
        let value = Value(json!({"other": "data"}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_expression_gte() {
        let node = SwitchNode::expression("${count} >= 10");

        let value = Value(json!({"count": 10}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"count": 15}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"count": 5}));
        assert!(!node.evaluate(&value));
    }

    #[test]
    fn switch_expression_not_equals() {
        let node = SwitchNode::expression("${status} != \"error\"");

        let value = Value(json!({"status": "success"}));
        assert!(node.evaluate(&value));

        let value = Value(json!({"status": "error"}));
        assert!(!node.evaluate(&value));
    }
}
