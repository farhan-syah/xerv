//! Loop node (controlled iteration).
//!
//! Provides controlled iteration with configurable exit conditions.
//! Loop back-edges must be declared in the flow graph to avoid
//! cycle detection errors.

use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// Exit condition for the loop.
#[derive(Debug, Clone)]
pub enum LoopCondition {
    /// Exit after a fixed number of iterations.
    MaxIterations(u32),
    /// Exit when a field equals a value.
    UntilFieldEquals { field: String, value: String },
    /// Exit when a boolean field becomes true.
    UntilTrue { field: String },
    /// Exit when a boolean field becomes false.
    UntilFalse { field: String },
    /// Exit based on expression evaluation.
    UntilExpression(String),
}

impl Default for LoopCondition {
    fn default() -> Self {
        Self::MaxIterations(10)
    }
}

/// Loop node - controlled iteration.
///
/// Implements a loop construct for repeated execution. The loop
/// continues until an exit condition is met. Loop back-edges must
/// be declared in the flow topology to be excluded from cycle detection.
///
/// # Ports
/// - Input: "in" - Initial input / data from loop body
/// - Output: "continue" - Activated to continue the loop
/// - Output: "exit" - Activated when loop terminates
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   retry_loop:
///     type: std::loop
///     config:
///       max_iterations: 3
///       # Or: until_true: $.success
///       # Or: until_expression: "${iteration} >= 3 || ${result.success}"
///     inputs:
///       - from: start.out -> in
///       - from: retry_action.out -> in  # back-edge from loop body
///     outputs:
///       continue: -> retry_action.in
///       exit: -> done.in
/// ```
///
/// # Back-edge Declaration
/// ```yaml
/// topology:
///   loop_edges:
///     - from: retry_action
///       to: retry_loop  # This edge is treated as a back-edge
/// ```
#[derive(Debug)]
pub struct LoopNode {
    /// Exit condition.
    condition: LoopCondition,
    /// Maximum iterations (safety limit).
    max_iterations: u32,
}

impl LoopNode {
    /// Create a loop with max iterations exit condition.
    pub fn with_max_iterations(max: u32) -> Self {
        Self {
            condition: LoopCondition::MaxIterations(max),
            max_iterations: max,
        }
    }

    /// Create a loop that runs until a field equals a value.
    pub fn until_field_equals(
        field: impl Into<String>,
        value: impl Into<String>,
        max_iterations: u32,
    ) -> Self {
        Self {
            condition: LoopCondition::UntilFieldEquals {
                field: field.into(),
                value: value.into(),
            },
            max_iterations,
        }
    }

    /// Create a loop that runs until a boolean field is true.
    pub fn until_true(field: impl Into<String>, max_iterations: u32) -> Self {
        Self {
            condition: LoopCondition::UntilTrue {
                field: field.into(),
            },
            max_iterations,
        }
    }

    /// Create a loop with a custom exit expression.
    pub fn with_expression(expr: impl Into<String>, max_iterations: u32) -> Self {
        Self {
            condition: LoopCondition::UntilExpression(expr.into()),
            max_iterations,
        }
    }

    /// Evaluate whether the loop should exit.
    ///
    /// Returns `true` if the loop should exit, `false` to continue.
    /// The max_iterations limit is always checked as a safety mechanism.
    fn should_exit(&self, value: &Value, iteration: u32) -> bool {
        // Check max iterations safety limit
        if iteration >= self.max_iterations {
            tracing::debug!(
                iteration = iteration,
                max = self.max_iterations,
                "Loop hit max iterations safety limit"
            );
            return true;
        }

        // Check condition-specific exit
        match &self.condition {
            LoopCondition::MaxIterations(max) => {
                let should_exit = iteration >= *max;
                tracing::debug!(
                    iteration = iteration,
                    max = max,
                    should_exit = should_exit,
                    "Evaluating max_iterations"
                );
                should_exit
            }

            LoopCondition::UntilFieldEquals {
                field,
                value: expected,
            } => {
                let should_exit = value.field_equals(field, expected);
                tracing::debug!(
                    field = %field,
                    expected = %expected,
                    iteration = iteration,
                    should_exit = should_exit,
                    "Evaluated until_field_equals"
                );
                should_exit
            }

            LoopCondition::UntilTrue { field } => {
                let should_exit = value.field_is_true(field);
                tracing::debug!(
                    field = %field,
                    iteration = iteration,
                    should_exit = should_exit,
                    "Evaluated until_true"
                );
                should_exit
            }

            LoopCondition::UntilFalse { field } => {
                let should_exit = value.field_is_false(field);
                tracing::debug!(
                    field = %field,
                    iteration = iteration,
                    should_exit = should_exit,
                    "Evaluated until_false"
                );
                should_exit
            }

            LoopCondition::UntilExpression(expr) => {
                let should_exit = self.evaluate_expression(expr, value, iteration);
                tracing::debug!(
                    expr = %expr,
                    iteration = iteration,
                    should_exit = should_exit,
                    "Evaluated until_expression"
                );
                should_exit
            }
        }
    }

    /// Evaluate a simple expression for loop exit condition.
    ///
    /// Supports:
    /// - `${field} == "value"` -> field equality
    /// - `${field} > number` -> numeric comparison
    /// - `${field}` -> boolean check
    /// - `${iteration} >= number` -> iteration count check
    fn evaluate_expression(&self, expr: &str, value: &Value, iteration: u32) -> bool {
        let expr = expr.trim();

        // Handle special ${iteration} variable
        if expr.contains("${iteration}") {
            return self.evaluate_iteration_expr(expr, iteration);
        }

        // Try to parse comparison expressions
        if let Some((field, op, rhs)) = self.parse_comparison(expr) {
            match op {
                "==" | "=" => {
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
            // Just a field reference - check if truthy (exit when true)
            value.field_is_true(&field)
        } else {
            tracing::warn!(expr = %expr, "Unrecognized expression format");
            false
        }
    }

    /// Evaluate an expression containing ${iteration}.
    fn evaluate_iteration_expr(&self, expr: &str, iteration: u32) -> bool {
        // Parse patterns like "${iteration} >= 3" or "${iteration} == 5"
        let operators = [">=", "<=", "==", "!=", ">", "<"];

        for op in operators {
            if let Some(pos) = expr.find(op) {
                let lhs = expr[..pos].trim();
                let rhs = expr[pos + op.len()..].trim();

                if lhs == "${iteration}" {
                    if let Ok(threshold) = rhs.parse::<u32>() {
                        return match op {
                            ">=" => iteration >= threshold,
                            "<=" => iteration <= threshold,
                            "==" => iteration == threshold,
                            "!=" => iteration != threshold,
                            ">" => iteration > threshold,
                            "<" => iteration < threshold,
                            _ => false,
                        };
                    }
                }
            }
        }

        false
    }

    /// Parse a comparison expression like "${field} > 0.5"
    fn parse_comparison<'a>(&self, expr: &'a str) -> Option<(String, &'a str, &'a str)> {
        let operators = [">=", "<=", "==", "!=", ">", "<", "="];

        for op in operators {
            if let Some(pos) = expr.find(op) {
                let lhs = expr[..pos].trim();
                let rhs = expr[pos + op.len()..].trim();

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

        // Skip ${iteration} - it's handled separately
        if s == "${iteration}" {
            return None;
        }

        // ${field.path} format
        if s.starts_with("${") && s.ends_with('}') {
            return Some(s[2..s.len() - 1].to_string());
        }

        // $.field.path format
        if let Some(stripped) = s.strip_prefix("$.") {
            return Some(stripped.to_string());
        }

        // Plain field name
        if !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return Some(s.to_string());
        }

        None
    }
}

impl Node for LoopNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "loop")
            .with_description("Controlled iteration with configurable exit conditions")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("continue", PortDirection::Output, "Any")
                    .with_description("Activated to continue loop iteration"),
                Port::named("exit", PortDirection::Output, "Any")
                    .with_description("Activated when loop terminates"),
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

            // In a real implementation, we'd track iteration count in the trace state.
            // The executor would call ctx.loop_iteration() to get/increment.
            // For now, we demonstrate the structure with a placeholder.
            // TODO: Implement iteration tracking in executor/trace state
            let iteration = 0_u32; // Would be: ctx.get_loop_iteration(node_id)

            let should_exit = self.should_exit(&value, iteration);

            tracing::debug!(
                iteration = iteration,
                should_exit = should_exit,
                "Loop iteration decision"
            );

            if should_exit {
                // Exit the loop
                Ok(NodeOutput::new("exit", input))
            } else {
                // Continue looping
                Ok(NodeOutput::new("continue", input))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loop_node_info() {
        let node = LoopNode::with_max_iterations(5);
        let info = node.info();

        assert_eq!(info.name, "std::loop");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 3);
        assert_eq!(info.outputs[0].name, "continue");
        assert_eq!(info.outputs[1].name, "exit");
    }

    #[test]
    fn loop_max_iterations() {
        let node = LoopNode::with_max_iterations(3);
        let value = Value::null();

        // Should not exit on iteration 0, 1, 2
        assert!(!node.should_exit(&value, 0));
        assert!(!node.should_exit(&value, 1));
        assert!(!node.should_exit(&value, 2));

        // Should exit on iteration 3+
        assert!(node.should_exit(&value, 3));
        assert!(node.should_exit(&value, 10));
    }

    #[test]
    fn loop_condition_default() {
        let condition = LoopCondition::default();
        assert!(matches!(condition, LoopCondition::MaxIterations(10)));
    }

    #[test]
    fn loop_until_field_equals() {
        let node = LoopNode::until_field_equals("status", "complete", 100);

        // Should continue when status != "complete"
        let value = Value(json!({"status": "pending"}));
        assert!(!node.should_exit(&value, 0));

        // Should exit when status == "complete"
        let value = Value(json!({"status": "complete"}));
        assert!(node.should_exit(&value, 0));
    }

    #[test]
    fn loop_until_true() {
        let node = LoopNode::until_true("done", 100);

        // Should continue when done == false
        let value = Value(json!({"done": false}));
        assert!(!node.should_exit(&value, 0));

        // Should exit when done == true
        let value = Value(json!({"done": true}));
        assert!(node.should_exit(&value, 0));
    }

    #[test]
    fn loop_until_false() {
        let node = LoopNode {
            condition: LoopCondition::UntilFalse {
                field: "running".to_string(),
            },
            max_iterations: 100,
        };

        // Should continue when running == true
        let value = Value(json!({"running": true}));
        assert!(!node.should_exit(&value, 0));

        // Should exit when running == false
        let value = Value(json!({"running": false}));
        assert!(node.should_exit(&value, 0));
    }

    #[test]
    fn loop_expression_iteration_count() {
        let node = LoopNode::with_expression("${iteration} >= 3", 100);

        let value = Value::null();

        // Should continue when iteration < 3
        assert!(!node.should_exit(&value, 0));
        assert!(!node.should_exit(&value, 1));
        assert!(!node.should_exit(&value, 2));

        // Should exit when iteration >= 3
        assert!(node.should_exit(&value, 3));
        assert!(node.should_exit(&value, 5));
    }

    #[test]
    fn loop_expression_field_comparison() {
        let node = LoopNode::with_expression("${error_count} > 5", 100);

        // Should continue when error_count <= 5
        let value = Value(json!({"error_count": 3}));
        assert!(!node.should_exit(&value, 0));

        // Should exit when error_count > 5
        let value = Value(json!({"error_count": 7}));
        assert!(node.should_exit(&value, 0));
    }

    #[test]
    fn loop_safety_limit_overrides_condition() {
        // Even if condition says continue, safety limit kicks in
        let node = LoopNode::until_field_equals("status", "complete", 5);

        // status is never "complete", but safety limit is 5
        let value = Value(json!({"status": "pending"}));

        assert!(!node.should_exit(&value, 0));
        assert!(!node.should_exit(&value, 4));
        assert!(node.should_exit(&value, 5)); // safety limit
    }

    #[test]
    fn loop_nested_field_access() {
        let node = LoopNode::until_field_equals("result.status", "success", 100);

        let value = Value(json!({"result": {"status": "pending"}}));
        assert!(!node.should_exit(&value, 0));

        let value = Value(json!({"result": {"status": "success"}}));
        assert!(node.should_exit(&value, 0));
    }

    #[test]
    fn loop_expression_boolean_field() {
        let node = LoopNode::with_expression("${is_valid}", 100);

        // Should continue when is_valid is false
        let value = Value(json!({"is_valid": false}));
        assert!(!node.should_exit(&value, 0));

        // Should exit when is_valid is true
        let value = Value(json!({"is_valid": true}));
        assert!(node.should_exit(&value, 0));
    }

    #[test]
    fn loop_expression_equality() {
        let node = LoopNode::with_expression("${state} == \"done\"", 100);

        let value = Value(json!({"state": "running"}));
        assert!(!node.should_exit(&value, 0));

        let value = Value(json!({"state": "done"}));
        assert!(node.should_exit(&value, 0));
    }
}
