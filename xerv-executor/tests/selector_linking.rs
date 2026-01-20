//! Integration tests for selector parsing and linking.
//!
//! Tests the compilation of ${node.field} expressions to memory offsets.

use xerv_core::traits::{FieldInfo, TypeInfo};
use xerv_core::types::NodeId;
use xerv_executor::linker::{Linker, NodeSchema, Selector, SelectorParser};

fn create_fraud_result_schema() -> TypeInfo {
    TypeInfo::new("FraudResult", 1)
        .with_hash(0x12345678)
        .with_size(24)
        .with_fields(vec![
            FieldInfo::new("score", "f64").with_offset(0).with_size(8),
            FieldInfo::new("risk_level", "u8")
                .with_offset(8)
                .with_size(1),
            FieldInfo::new("reason_code", "u32")
                .with_offset(12)
                .with_size(4),
            FieldInfo::new("confidence", "f32")
                .with_offset(16)
                .with_size(4),
        ])
        .stable()
}

fn create_order_schema() -> TypeInfo {
    TypeInfo::new("Order", 1)
        .with_hash(0xABCDEF01)
        .with_size(32)
        .with_fields(vec![
            FieldInfo::new("order_id", "u64")
                .with_offset(0)
                .with_size(8),
            FieldInfo::new("amount", "f64").with_offset(8).with_size(8),
            FieldInfo::new("currency", "u32")
                .with_offset(16)
                .with_size(4),
            FieldInfo::new("status", "u8").with_offset(20).with_size(1),
        ])
        .stable()
}

fn create_config_schema() -> TypeInfo {
    TypeInfo::new("PipelineConfig", 1)
        .with_hash(0x99887766)
        .with_size(24)
        .with_fields(vec![
            FieldInfo::new("min_order_value", "f64")
                .with_offset(0)
                .with_size(8),
            FieldInfo::new("fraud_threshold", "f64")
                .with_offset(8)
                .with_size(8),
            FieldInfo::new("max_retries", "u32")
                .with_offset(16)
                .with_size(4),
        ])
        .stable()
}

// Parser Tests

#[test]
fn parse_simple_selector() {
    let selector = SelectorParser::parse_expression("fraud_check.score").unwrap();
    assert_eq!(selector.root, "fraud_check");
    assert_eq!(selector.path, vec!["score"]);
}

#[test]
fn parse_deep_nested_selector() {
    let selector = SelectorParser::parse_expression("pipeline.config.settings.timeout").unwrap();
    assert_eq!(selector.root, "pipeline");
    assert_eq!(selector.path, vec!["config", "settings", "timeout"]);
}

#[test]
fn parse_multiple_selectors_in_template() {
    let template = "Order ${order.order_id} has fraud score ${fraud.score} with confidence ${fraud.confidence}";
    let selectors = SelectorParser::parse_all(template).unwrap();

    assert_eq!(selectors.len(), 3);
    assert_eq!(selectors[0].root, "order");
    assert_eq!(selectors[0].path, vec!["order_id"]);
    assert_eq!(selectors[1].root, "fraud");
    assert_eq!(selectors[1].path, vec!["score"]);
    assert_eq!(selectors[2].root, "fraud");
    assert_eq!(selectors[2].path, vec!["confidence"]);
}

#[test]
fn parse_selector_with_underscores() {
    let selector = SelectorParser::parse_expression("fraud_check.risk_level").unwrap();
    assert_eq!(selector.root, "fraud_check");
    assert_eq!(selector.path, vec!["risk_level"]);
}

#[test]
fn parse_empty_selector_fails() {
    let result = SelectorParser::parse_expression("");
    assert!(result.is_err());
}

#[test]
fn parse_invalid_identifier_fails() {
    let result = SelectorParser::parse_expression("123invalid.field");
    assert!(result.is_err());
}

#[test]
fn parse_unclosed_selector_fails() {
    let result = SelectorParser::parse_all("Value is ${unclosed");
    assert!(result.is_err());
}

#[test]
fn interpolate_template() {
    let template = "Score: ${fraud.score}, Level: ${fraud.risk_level}";
    let result = SelectorParser::interpolate(template, |selector| {
        if selector.root == "fraud" {
            match selector.path[0].as_str() {
                "score" => Ok("0.95".to_string()),
                "risk_level" => Ok("HIGH".to_string()),
                _ => Err(xerv_core::error::XervError::SelectorSyntax {
                    selector: selector.raw.clone(),
                    cause: "Unknown field".to_string(),
                }),
            }
        } else {
            Err(xerv_core::error::XervError::SelectorSyntax {
                selector: selector.raw.clone(),
                cause: "Unknown node".to_string(),
            })
        }
    })
    .unwrap();

    assert_eq!(result, "Score: 0.95, Level: HIGH");
}

// Linker Tests

#[test]
fn linker_compile_simple_field() {
    let mut linker = Linker::new();

    let schema = create_fraud_result_schema();
    let node_schema = NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", schema);
    linker.register_node("fraud_check", node_schema);

    let selector = Selector::new("fraud_check", vec!["score".to_string()]);
    let compiled = linker.compile(&selector).unwrap();

    assert_eq!(compiled.source_node, NodeId::new(1));
    assert_eq!(compiled.source_port, "out");
    assert!(compiled.is_static());
    assert_eq!(compiled.total_offset(), 0);
    assert_eq!(compiled.final_type(), "f64");
    assert_eq!(compiled.final_size(), 8);
}

#[test]
fn linker_compile_field_with_offset() {
    let mut linker = Linker::new();

    let schema = create_fraud_result_schema();
    let node_schema = NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", schema);
    linker.register_node("fraud_check", node_schema);

    let selector = Selector::new("fraud_check", vec!["reason_code".to_string()]);
    let compiled = linker.compile(&selector).unwrap();

    assert!(compiled.is_static());
    assert_eq!(compiled.total_offset(), 12);
    assert_eq!(compiled.final_type(), "u32");
    assert_eq!(compiled.final_size(), 4);
}

#[test]
fn linker_compile_pipeline_config() {
    let mut linker = Linker::new();

    let config_schema = create_config_schema();
    linker.register_pipeline_config(config_schema);

    let selector = Selector::new(
        "pipeline",
        vec!["config".to_string(), "fraud_threshold".to_string()],
    );
    let compiled = linker.compile(&selector).unwrap();

    assert_eq!(compiled.source_port, "__config__");
    assert!(compiled.is_static());
    assert_eq!(compiled.total_offset(), 8);
    assert_eq!(compiled.final_type(), "f64");
}

#[test]
fn linker_compile_unknown_node_fails() {
    let linker = Linker::new();

    let selector = Selector::new("unknown_node", vec!["field".to_string()]);
    let result = linker.compile(&selector);

    assert!(result.is_err());
}

#[test]
fn linker_compile_unknown_field_fails() {
    let mut linker = Linker::new();

    let schema = create_fraud_result_schema();
    let node_schema = NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", schema);
    linker.register_node("fraud_check", node_schema);

    let selector = Selector::new("fraud_check", vec!["nonexistent_field".to_string()]);
    let result = linker.compile(&selector);

    assert!(result.is_err());
}

#[test]
fn linker_multiple_nodes() {
    let mut linker = Linker::new();

    let fraud_schema = create_fraud_result_schema();
    let order_schema = create_order_schema();

    let fraud_node =
        NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", fraud_schema);
    let order_node =
        NodeSchema::new(NodeId::new(2), "order_parser").with_output("out", order_schema);

    linker.register_node("fraud_check", fraud_node);
    linker.register_node("order_parser", order_node);

    // Compile selectors for both nodes
    let fraud_selector = Selector::new("fraud_check", vec!["score".to_string()]);
    let fraud_compiled = linker.compile(&fraud_selector).unwrap();
    assert_eq!(fraud_compiled.source_node, NodeId::new(1));

    let order_selector = Selector::new("order_parser", vec!["amount".to_string()]);
    let order_compiled = linker.compile(&order_selector).unwrap();
    assert_eq!(order_compiled.source_node, NodeId::new(2));
    assert_eq!(order_compiled.total_offset(), 8);
}

#[test]
fn linker_compile_template() {
    let mut linker = Linker::new();

    let schema = create_fraud_result_schema();
    let node_schema = NodeSchema::new(NodeId::new(1), "fraud").with_output("out", schema);
    linker.register_node("fraud", node_schema);

    let template = "Score: ${fraud.score}, Confidence: ${fraud.confidence}";
    let compiled = linker.compile_template(template).unwrap();

    assert_eq!(compiled.len(), 2);
    assert_eq!(compiled[0].fields()[0].name, "score");
    assert_eq!(compiled[1].fields()[0].name, "confidence");
}

#[test]
fn compiled_selector_is_primitive() {
    let mut linker = Linker::new();

    let schema = create_fraud_result_schema();
    let node_schema = NodeSchema::new(NodeId::new(1), "fraud").with_output("out", schema);
    linker.register_node("fraud", node_schema);

    let selector = Selector::new("fraud", vec!["score".to_string()]);
    let compiled = linker.compile(&selector).unwrap();

    assert!(compiled.is_primitive());
    assert_eq!(compiled.offset(), Some(0));
}

#[test]
fn selector_full_path() {
    let selector = Selector::new(
        "pipeline",
        vec!["config".to_string(), "max_retries".to_string()],
    );

    assert_eq!(selector.full_path(), "pipeline.config.max_retries");
    assert_eq!(selector.field_path(), "config.max_retries");
}

#[test]
fn selector_is_pipeline_config() {
    let config_selector = Selector::new(
        "pipeline",
        vec!["config".to_string(), "timeout".to_string()],
    );
    assert!(config_selector.is_pipeline_config());

    let node_selector = Selector::new("fraud", vec!["score".to_string()]);
    assert!(!node_selector.is_pipeline_config());

    let pipeline_non_config = Selector::new("pipeline", vec!["status".to_string()]);
    assert!(!pipeline_non_config.is_pipeline_config());
}
