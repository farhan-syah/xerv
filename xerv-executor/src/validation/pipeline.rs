//! Pipeline validation reports for API usage.

use crate::loader::FlowBuilder;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use xerv_core::flow::{FlowDefinition, FlowValidator, ValidationError, ValidationLimits};

/// A single validation issue.
#[derive(Debug, Serialize)]
pub struct ValidationIssue {
    /// Machine-readable kind (e.g., "INVALID_REFERENCE").
    pub kind: String,
    /// Human-readable message.
    pub message: String,
    /// Optional location path in the flow definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Optional list of node IDs associated with the issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<String>>,
}

impl ValidationIssue {
    fn from_validation_error(error: ValidationError) -> Self {
        Self {
            kind: error.kind.to_string(),
            message: error.message,
            location: Some(error.location),
            nodes: None,
        }
    }

    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            location: None,
            nodes: None,
        }
    }

    fn with_nodes(kind: &str, message: impl Into<String>, nodes: Vec<String>) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            location: None,
            nodes: Some(nodes),
        }
    }

    fn with_location(kind: &str, message: impl Into<String>, location: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            location: Some(location.into()),
            nodes: None,
        }
    }
}

/// A validation report returned by API endpoints.
#[derive(Debug, Serialize)]
pub struct ValidationReport {
    /// Whether the flow is valid for the current context.
    pub valid: bool,
    /// Hard errors that block the action.
    pub errors: Vec<ValidationIssue>,
    /// Non-blocking warnings.
    pub warnings: Vec<ValidationIssue>,
}

/// Validate a flow for editing/saving (warnings are non-blocking).
pub fn validate_for_editing(flow: &FlowDefinition, limits: &ValidationLimits) -> ValidationReport {
    let mut errors = collect_validation_errors(flow, limits);

    if let Some(issue) = build_error(flow) {
        errors.push(issue);
    }

    let warnings = collect_warnings(flow);

    ValidationReport {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

/// Validate a flow for deployment (unreachable nodes are errors).
pub fn validate_for_deployment(
    flow: &FlowDefinition,
    limits: &ValidationLimits,
) -> ValidationReport {
    let mut errors = collect_validation_errors(flow, limits);

    if let Some(issue) = build_error(flow) {
        errors.push(issue);
    }

    let (reachable, enabled_nodes, enabled_triggers) = compute_reachability(flow);
    if enabled_triggers.is_empty() {
        errors.push(ValidationIssue::new(
            "NO_TRIGGER",
            "Pipeline must have at least one enabled trigger",
        ));
    }

    let unreachable = enabled_nodes
        .iter()
        .filter(|node_id| !reachable.contains(*node_id))
        .cloned()
        .collect::<Vec<_>>();

    if !unreachable.is_empty() {
        errors.push(ValidationIssue::with_nodes(
            "UNREACHABLE_NODES",
            format!("Nodes unreachable from trigger: {:?}", unreachable),
            unreachable,
        ));
    }

    ValidationReport {
        valid: errors.is_empty(),
        errors,
        warnings: Vec::new(),
    }
}

fn collect_validation_errors(
    flow: &FlowDefinition,
    limits: &ValidationLimits,
) -> Vec<ValidationIssue> {
    FlowValidator::with_limits(limits.clone())
        .validate(flow)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(ValidationIssue::from_validation_error)
        .collect()
}

fn build_error(flow: &FlowDefinition) -> Option<ValidationIssue> {
    FlowBuilder::new()
        .build(flow)
        .err()
        .map(|err| ValidationIssue::new("BUILD_ERROR", err.to_string()))
}

fn collect_warnings(flow: &FlowDefinition) -> Vec<ValidationIssue> {
    let mut warnings = Vec::new();

    let (reachable, enabled_nodes, _enabled_triggers) = compute_reachability(flow);

    let orphaned = find_orphaned_nodes(flow, &enabled_nodes);
    for node_id in orphaned {
        warnings.push(ValidationIssue::with_location(
            "ORPHANED_NODE",
            format!("Node '{}' has no connections", node_id),
            format!("nodes.{}", node_id),
        ));
    }

    let unreachable = enabled_nodes
        .iter()
        .filter(|node_id| !reachable.contains(*node_id))
        .cloned()
        .collect::<Vec<_>>();

    if !unreachable.is_empty() {
        warnings.push(ValidationIssue::with_nodes(
            "UNREACHABLE_NODES",
            format!("Nodes unreachable from trigger: {:?}", unreachable),
            unreachable,
        ));
    }

    warnings
}

fn compute_reachability(
    flow: &FlowDefinition,
) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let enabled_nodes: HashSet<String> =
        flow.enabled_nodes().map(|(id, _)| id.to_string()).collect();
    let enabled_triggers: HashSet<String> = flow.enabled_triggers().map(|t| t.id.clone()).collect();

    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for id in enabled_nodes.iter().chain(enabled_triggers.iter()) {
        adjacency.entry(id.clone()).or_default();
    }

    for edge in &flow.edges {
        let from = edge.from_node();
        let to = edge.to_node();

        let from_enabled = enabled_nodes.contains(from) || enabled_triggers.contains(from);
        let to_enabled = enabled_nodes.contains(to);

        if from_enabled && to_enabled {
            adjacency
                .entry(from.to_string())
                .or_default()
                .push(to.to_string());
        }
    }

    let mut reachable: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = enabled_triggers.iter().cloned().collect();

    while let Some(node_id) = queue.pop_front() {
        if reachable.insert(node_id.clone()) {
            if let Some(children) = adjacency.get(&node_id) {
                for child in children {
                    queue.push_back(child.clone());
                }
            }
        }
    }

    (reachable, enabled_nodes, enabled_triggers)
}

fn find_orphaned_nodes(flow: &FlowDefinition, enabled_nodes: &HashSet<String>) -> Vec<String> {
    let mut incoming: HashMap<&str, usize> =
        enabled_nodes.iter().map(|id| (id.as_str(), 0)).collect();
    let mut outgoing: HashMap<&str, usize> =
        enabled_nodes.iter().map(|id| (id.as_str(), 0)).collect();

    for edge in &flow.edges {
        let from = edge.from_node();
        let to = edge.to_node();

        if outgoing.contains_key(from) {
            if let Some(count) = outgoing.get_mut(from) {
                *count += 1;
            }
        }
        if incoming.contains_key(to) {
            if let Some(count) = incoming.get_mut(to) {
                *count += 1;
            }
        }
    }

    enabled_nodes
        .iter()
        .filter(|node_id| {
            let in_count = incoming.get(node_id.as_str()).copied().unwrap_or(0);
            let out_count = outgoing.get(node_id.as_str()).copied().unwrap_or(0);
            in_count == 0 && out_count == 0
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xerv_core::flow::{EdgeDefinition, NodeDefinition, TriggerDefinition};

    fn base_flow() -> FlowDefinition {
        FlowDefinition::new("test_flow").with_trigger(TriggerDefinition::new("webhook", "webhook"))
    }

    #[test]
    fn editing_allows_orphan_nodes_with_warnings() {
        let flow = base_flow().with_node("orphan", NodeDefinition::new("std::log"));
        let report = validate_for_editing(&flow, &ValidationLimits::default());

        assert!(report.valid);
        assert!(!report.warnings.is_empty());
        assert!(report.warnings.iter().any(|w| w.kind == "ORPHANED_NODE"));
    }

    #[test]
    fn deployment_fails_on_unreachable_nodes() {
        let flow = base_flow().with_node("orphan", NodeDefinition::new("std::log"));
        let report = validate_for_deployment(&flow, &ValidationLimits::default());

        assert!(!report.valid);
        assert!(report.errors.iter().any(|e| e.kind == "UNREACHABLE_NODES"));
    }

    #[test]
    fn deployment_requires_trigger() {
        let flow = FlowDefinition::new("no_trigger")
            .with_node("a", NodeDefinition::new("std::log"))
            .with_node("b", NodeDefinition::new("std::log"))
            .with_edge(EdgeDefinition::new("a", "b"));
        let report = validate_for_deployment(&flow, &ValidationLimits::default());

        assert!(!report.valid);
        assert!(report.errors.iter().any(|e| e.kind == "NO_TRIGGER"));
    }

    #[test]
    fn deployment_accepts_reachable_graph() {
        let flow = base_flow()
            .with_node("a", NodeDefinition::new("std::log"))
            .with_edge(EdgeDefinition::new("webhook", "a"));
        let report = validate_for_deployment(&flow, &ValidationLimits::default());

        assert!(report.valid);
        assert!(report.errors.is_empty());
    }
}
