use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ValidationResult {
    valid: bool,
    errors: Vec<String>,
    warnings: Vec<String>,
}

#[wasm_bindgen]
impl ValidationResult {
    #[wasm_bindgen(getter)]
    pub fn valid(&self) -> bool {
        self.valid
    }

    #[wasm_bindgen(getter)]
    pub fn errors(&self) -> Vec<String> {
        self.errors.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PortSchema {
    name: String,
    type_name: String,
}

#[derive(Debug, Clone)]
struct NodeSchema {
    inputs: HashMap<String, PortSchema>,
    outputs: HashMap<String, PortSchema>,
}

static SCHEMA_REGISTRY: Lazy<HashMap<String, NodeSchema>> = Lazy::new(|| {
    let mut registry = HashMap::new();

    registry.insert(
        "std::log".to_string(),
        NodeSchema {
            inputs: HashMap::from([(
                "input".to_string(),
                PortSchema {
                    name: "input".to_string(),
                    type_name: "Any".to_string(),
                },
            )]),
            outputs: HashMap::from([(
                "output".to_string(),
                PortSchema {
                    name: "output".to_string(),
                    type_name: "Any".to_string(),
                },
            )]),
        },
    );

    registry.insert(
        "std::switch".to_string(),
        NodeSchema {
            inputs: HashMap::from([(
                "input".to_string(),
                PortSchema {
                    name: "input".to_string(),
                    type_name: "Any".to_string(),
                },
            )]),
            outputs: HashMap::from([
                (
                    "true".to_string(),
                    PortSchema {
                        name: "true".to_string(),
                        type_name: "Any".to_string(),
                    },
                ),
                (
                    "false".to_string(),
                    PortSchema {
                        name: "false".to_string(),
                        type_name: "Any".to_string(),
                    },
                ),
            ]),
        },
    );

    registry.insert(
        "std::http".to_string(),
        NodeSchema {
            inputs: HashMap::new(),
            outputs: HashMap::from([
                (
                    "response".to_string(),
                    PortSchema {
                        name: "response".to_string(),
                        type_name: "Any".to_string(),
                    },
                ),
                (
                    "status_code".to_string(),
                    PortSchema {
                        name: "status_code".to_string(),
                        type_name: "Number".to_string(),
                    },
                ),
            ]),
        },
    );

    registry
});

fn is_compatible(source: &str, target: &str) -> bool {
    source == "Any" || target == "Any" || source == target
}

#[wasm_bindgen]
pub fn validate_edge(
    source_type: &str,
    source_output: &str,
    target_type: &str,
    target_input: &str,
) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let source_schema = match SCHEMA_REGISTRY.get(source_type) {
        Some(schema) => schema,
        None => {
            warnings.push(format!(
                "Unknown source node type '{}'; treating output as Any",
                source_type
            ));
            return ValidationResult {
                valid: true,
                errors,
                warnings,
            };
        }
    };

    let target_schema = match SCHEMA_REGISTRY.get(target_type) {
        Some(schema) => schema,
        None => {
            warnings.push(format!(
                "Unknown target node type '{}'; treating input as Any",
                target_type
            ));
            return ValidationResult {
                valid: true,
                errors,
                warnings,
            };
        }
    };

    let source_port = match source_schema.outputs.get(source_output) {
        Some(port) => port,
        None => {
            warnings.push(format!(
                "Unknown output '{}' on node type '{}'",
                source_output, source_type
            ));
            return ValidationResult {
                valid: true,
                errors,
                warnings,
            };
        }
    };

    let target_port = match target_schema.inputs.get(target_input) {
        Some(port) => port,
        None => {
            warnings.push(format!(
                "Unknown input '{}' on node type '{}'",
                target_input, target_type
            ));
            return ValidationResult {
                valid: true,
                errors,
                warnings,
            };
        }
    };

    if is_compatible(&source_port.type_name, &target_port.type_name) {
        ValidationResult {
            valid: true,
            errors,
            warnings,
        }
    } else {
        errors.push(format!(
            "Type mismatch: {} produces '{}', but {} expects '{}'",
            source_type, source_port.type_name, target_type, target_port.type_name
        ));
        ValidationResult {
            valid: false,
            errors,
            warnings,
        }
    }
}

#[wasm_bindgen]
pub fn validate_selector(expression: &str) -> ValidationResult {
    let mut errors = Vec::new();

    if expression.contains("${") {
        let mut in_selector = false;
        let mut depth = 0;
        let mut current = String::new();

        for c in expression.chars() {
            match c {
                '$' => {}
                '{' if expression.contains("${") => {
                    in_selector = true;
                    depth += 1;
                    current.clear();
                }
                '}' if in_selector => {
                    depth -= 1;
                    if depth == 0 {
                        in_selector = false;
                        if !current.contains('.') || current.ends_with('.') {
                            errors.push("Selector must include a field reference".to_string());
                        }
                    }
                }
                _ if in_selector => {
                    current.push(c);
                }
                _ => {}
            }
        }

        if depth != 0 {
            errors.push("Unbalanced braces in selector".to_string());
        }
    }

    ValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings: Vec::new(),
    }
}

#[derive(Debug, Serialize)]
struct SelectorToken {
    kind: String,
    value: String,
}

#[wasm_bindgen]
pub fn format_selector(expression: &str) -> String {
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    let mut in_selector = false;

    let mut chars = expression.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            if !buffer.is_empty() {
                tokens.push(SelectorToken {
                    kind: "text".to_string(),
                    value: buffer.clone(),
                });
                buffer.clear();
            }
            in_selector = true;
            buffer.push(c);
        } else if c == '}' && in_selector {
            buffer.push(c);
            tokens.push(SelectorToken {
                kind: "selector".to_string(),
                value: buffer.clone(),
            });
            buffer.clear();
            in_selector = false;
        } else {
            buffer.push(c);
        }
    }

    if !buffer.is_empty() {
        tokens.push(SelectorToken {
            kind: if in_selector {
                "selector".to_string()
            } else {
                "text".to_string()
            },
            value: buffer,
        });
    }

    serde_json::to_string(&tokens).unwrap_or_else(|_| "[]".to_string())
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Pipeline {
    id: String,
    name: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Node {
    id: String,
    #[serde(rename = "type")]
    node_type: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Edge {
    id: String,
    source: String,
    target: String,
}

#[wasm_bindgen]
pub fn validate_pipeline(pipeline_json: &str) -> ValidationResult {
    let pipeline: Pipeline = match serde_json::from_str(pipeline_json) {
        Ok(p) => p,
        Err(e) => {
            return ValidationResult {
                valid: false,
                errors: vec![format!("Invalid JSON: {}", e)],
                warnings: Vec::new(),
            };
        }
    };

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if has_cycle(&pipeline) {
        errors.push("Cycle detected in pipeline".to_string());
    }

    let orphans = find_orphaned_nodes(&pipeline);
    for node_id in orphans {
        warnings.push(format!("Node '{}' has no connections", node_id));
    }

    ValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

fn has_cycle(pipeline: &Pipeline) -> bool {
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &pipeline.edges {
        graph
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();

    for node in &pipeline.nodes {
        if dfs_cycle(node.id.as_str(), &graph, &mut visiting, &mut visited) {
            return true;
        }
    }

    false
}

fn dfs_cycle(
    node: &str,
    graph: &HashMap<&str, Vec<&str>>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
) -> bool {
    if visited.contains(node) {
        return false;
    }

    if visiting.contains(node) {
        return true;
    }

    visiting.insert(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for neighbor in neighbors {
            if dfs_cycle(neighbor, graph, visiting, visited) {
                return true;
            }
        }
    }

    visiting.remove(node);
    visited.insert(node.to_string());
    false
}

fn find_orphaned_nodes(pipeline: &Pipeline) -> Vec<String> {
    let mut connected = HashSet::new();

    for edge in &pipeline.edges {
        connected.insert(edge.source.as_str());
        connected.insert(edge.target.as_str());
    }

    pipeline
        .nodes
        .iter()
        .filter(|node| !connected.contains(node.id.as_str()))
        .map(|node| node.id.clone())
        .collect()
}
