//! Snapshot testing utilities.
//!
//! Snapshot testing captures expected output and compares against it in future runs.
//! This is useful for testing complex outputs like serialized data structures.

use serde::{Serialize, de::DeserializeOwned};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Get the snapshot directory for the current test.
fn snapshot_dir() -> PathBuf {
    // Default to tests/snapshots in the current crate
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

/// Get the path for a named snapshot.
fn snapshot_path(name: &str) -> PathBuf {
    snapshot_dir().join(format!("{}.snap", name))
}

/// Check if snapshots should be updated.
fn should_update() -> bool {
    env::var("UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

/// Assert that a value matches a snapshot.
///
/// If the snapshot doesn't exist, it will be created.
/// If `UPDATE_SNAPSHOTS=1` is set, the snapshot will be updated.
///
/// # Example
///
/// ```ignore
/// use xerv_core::testing::assert_snapshot;
/// use serde_json::json;
///
/// let output = json!({
///     "status": "success",
///     "data": [1, 2, 3]
/// });
///
/// // First run: creates tests/snapshots/my_output.snap
/// // Subsequent runs: compares against the snapshot
/// assert_snapshot("my_output", &output);
/// ```
///
/// To update snapshots, run tests with:
/// ```bash
/// UPDATE_SNAPSHOTS=1 cargo test
/// ```
pub fn assert_snapshot<T>(name: &str, value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let path = snapshot_path(name);
    let actual_json = serde_json::to_string_pretty(value).expect("Failed to serialize value");

    if should_update() || !path.exists() {
        // Create directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create snapshot directory");
        }

        // Write the snapshot
        fs::write(&path, &actual_json).expect("Failed to write snapshot");

        if !should_update() {
            println!("Created new snapshot: {}", path.display());
        } else {
            println!("Updated snapshot: {}", path.display());
        }
        return;
    }

    // Read and compare
    let expected_json = fs::read_to_string(&path).expect("Failed to read snapshot");

    if actual_json != expected_json {
        let expected: T =
            serde_json::from_str(&expected_json).expect("Failed to deserialize snapshot");

        if value == &expected {
            // Values are equal but JSON differs (e.g., whitespace)
            // This is fine, don't fail
            return;
        }

        // Generate diff for error message
        let diff = generate_diff(&expected_json, &actual_json);

        panic!(
            "Snapshot mismatch for '{}'!\n\n\
            Run with UPDATE_SNAPSHOTS=1 to update.\n\n\
            Diff:\n{}\n\n\
            Snapshot file: {}",
            name,
            diff,
            path.display()
        );
    }
}

/// Generate a simple diff between two strings.
fn generate_diff(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();

    let mut diff = String::new();
    let max_lines = expected_lines.len().max(actual_lines.len());

    for i in 0..max_lines {
        let exp = expected_lines.get(i).copied().unwrap_or("");
        let act = actual_lines.get(i).copied().unwrap_or("");

        if exp == act {
            diff.push_str(&format!("  {}\n", exp));
        } else {
            if !exp.is_empty() {
                diff.push_str(&format!("- {}\n", exp));
            }
            if !act.is_empty() {
                diff.push_str(&format!("+ {}\n", act));
            }
        }
    }

    diff
}

/// Assert that a JSON value matches a snapshot.
///
/// Convenience wrapper for JSON values.
pub fn assert_json_snapshot(name: &str, value: &serde_json::Value) {
    assert_snapshot(name, value);
}

/// Assert that a string matches a snapshot.
///
/// For simple string comparisons without JSON parsing.
pub fn assert_string_snapshot(name: &str, value: &str) {
    let path = snapshot_path(name);

    if should_update() || !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create snapshot directory");
        }
        fs::write(&path, value).expect("Failed to write snapshot");
        return;
    }

    let expected = fs::read_to_string(&path).expect("Failed to read snapshot");

    if value != expected {
        let diff = generate_diff(&expected, value);
        panic!(
            "String snapshot mismatch for '{}'!\n\n\
            Run with UPDATE_SNAPSHOTS=1 to update.\n\n\
            Diff:\n{}\n\n\
            Snapshot file: {}",
            name,
            diff,
            path.display()
        );
    }
}

/// Inline snapshot assertion (snapshot embedded in test).
///
/// Unlike file-based snapshots, this embeds the expected value directly
/// in the test source code. Useful for small, stable outputs.
///
/// # Example
///
/// ```ignore
/// use xerv_core::testing::assert_inline_snapshot;
///
/// let output = compute_something();
/// assert_inline_snapshot!(output, r#"{"result": 42}"#);
/// ```
#[macro_export]
macro_rules! assert_inline_snapshot {
    ($actual:expr, $expected:expr) => {{
        let actual = serde_json::to_string_pretty(&$actual).expect("Failed to serialize");
        let expected = $expected;
        if actual != expected {
            panic!(
                "Inline snapshot mismatch!\n\nExpected:\n{}\n\nActual:\n{}",
                expected, actual
            );
        }
    }};
}

/// Redact sensitive or non-deterministic values in snapshots.
///
/// # Example
///
/// ```
/// use xerv_core::testing::snapshot::redact_json;
/// use serde_json::json;
///
/// let output = json!({
///     "id": "abc123",
///     "timestamp": 1234567890,
///     "name": "test"
/// });
///
/// let redacted = redact_json(&output, &["id", "timestamp"]);
/// // redacted["id"] == "[REDACTED]"
/// // redacted["timestamp"] == "[REDACTED]"
/// // redacted["name"] == "test"
/// ```
pub fn redact_json(value: &serde_json::Value, keys_to_redact: &[&str]) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                if keys_to_redact.contains(&k.as_str()) {
                    new_map.insert(
                        k.clone(),
                        serde_json::Value::String("[REDACTED]".to_string()),
                    );
                } else {
                    new_map.insert(k.clone(), redact_json(v, keys_to_redact));
                }
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| redact_json(v, keys_to_redact)).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_redact_json() {
        let input = json!({
            "public": "visible",
            "secret": "hidden",
            "nested": {
                "timestamp": 12345,
                "data": "ok"
            },
            "array": [
                {"id": "a", "value": 1},
                {"id": "b", "value": 2}
            ]
        });

        let redacted = redact_json(&input, &["secret", "timestamp", "id"]);

        assert_eq!(redacted["public"], "visible");
        assert_eq!(redacted["secret"], "[REDACTED]");
        assert_eq!(redacted["nested"]["timestamp"], "[REDACTED]");
        assert_eq!(redacted["nested"]["data"], "ok");
        assert_eq!(redacted["array"][0]["id"], "[REDACTED]");
        assert_eq!(redacted["array"][0]["value"], 1);
    }

    #[test]
    fn test_generate_diff() {
        let expected = "line1\nline2\nline3";
        let actual = "line1\nmodified\nline3";

        let diff = generate_diff(expected, actual);
        assert!(diff.contains("- line2"));
        assert!(diff.contains("+ modified"));
    }
}
