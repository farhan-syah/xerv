//! Validate command - validate a flow YAML file.

use anyhow::Result;
use std::path::Path;
use xerv_core::flow::FlowDefinition;
use xerv_executor::loader::{FlowLoader, LoaderConfig, LoaderError};

/// Run the validate command.
pub async fn run(file: &str) -> Result<()> {
    let path = Path::new(file);

    if !path.exists() {
        anyhow::bail!("Flow file not found: {}", file);
    }

    tracing::info!(file = %file, "Validating flow");

    println!("Validation Results for: {}", file);
    println!("========================{}", "=".repeat(file.len()));
    println!();

    // First, try to parse the YAML
    let content =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;

    let definition: FlowDefinition = match serde_yaml::from_str(&content) {
        Ok(def) => def,
        Err(e) => {
            println!("✗ YAML PARSE ERROR:");
            println!("  {}", e);
            anyhow::bail!("YAML parsing failed");
        }
    };

    println!("✓ YAML syntax is valid");
    println!();

    // Run schema validation
    let mut has_errors = false;
    let mut has_warnings = false;

    match definition.validate() {
        Ok(()) => {
            println!("✓ Schema validation passed");
        }
        Err(errors) => {
            has_errors = true;
            println!("✗ Schema validation failed:");
            for error in &errors {
                println!("  - {}", error);
            }
        }
    }
    println!();

    // Try to build the flow graph
    let loader = FlowLoader::with_config(LoaderConfig {
        validate: false, // We already validated above
        link_selectors: false,
        optimize: false,
    });

    match loader.load_yaml(&content) {
        Ok(loaded) => {
            println!("✓ Flow graph construction passed");
            println!();

            // Print flow summary
            println!("Flow Summary:");
            println!("  Name: {}", loaded.name());
            println!("  Version: {}", loaded.version());
            println!("  Triggers: {}", loaded.definition.triggers.len());
            println!("  Nodes: {}", loaded.definition.nodes.len());
            println!("  Edges: {}", loaded.definition.edges.len());

            // Check for warnings
            let enabled_triggers: Vec<_> = loaded
                .definition
                .triggers
                .iter()
                .filter(|t| t.enabled)
                .collect();
            let enabled_nodes: Vec<_> = loaded
                .definition
                .nodes
                .iter()
                .filter(|(_, n)| n.enabled)
                .collect();

            if enabled_triggers.is_empty() {
                has_warnings = true;
                println!();
                println!("⚠ WARNING: Flow has no enabled triggers");
            }

            if enabled_nodes.is_empty() {
                has_warnings = true;
                println!();
                println!("⚠ WARNING: Flow has no enabled nodes");
            }

            // Check for disconnected nodes
            let connected_nodes: std::collections::HashSet<&str> = loaded
                .definition
                .edges
                .iter()
                .flat_map(|e| vec![e.from_node(), e.to_node()])
                .collect();

            for (id, node) in &loaded.definition.nodes {
                if node.enabled && !connected_nodes.contains(id.as_str()) {
                    has_warnings = true;
                    println!();
                    println!("⚠ WARNING: Node '{}' is not connected by any edge", id);
                }
            }

            // Try topological sort
            match loaded.execution_order() {
                Ok(order) => {
                    println!();
                    println!("✓ Topological sort passed ({} nodes)", order.len());
                }
                Err(e) => {
                    has_errors = true;
                    println!();
                    println!("✗ Topological sort failed: {}", e);
                }
            }
        }
        Err(LoaderError::Validation { errors }) => {
            has_errors = true;
            println!("✗ Flow graph validation failed:");
            for error in &errors {
                println!("  - {}", error);
            }
        }
        Err(LoaderError::Build { source }) => {
            has_errors = true;
            println!("✗ Flow graph construction failed:");
            println!("  {}", source);
        }
        Err(e) => {
            has_errors = true;
            println!("✗ Unexpected error:");
            println!("  {}", e);
        }
    }

    println!();
    println!("========================{}", "=".repeat(file.len()));

    if has_errors {
        println!("✗ Validation FAILED");
        anyhow::bail!("Flow validation failed");
    } else if has_warnings {
        println!("⚠ Validation passed with warnings");
    } else {
        println!("✓ Validation PASSED");
    }

    Ok(())
}
