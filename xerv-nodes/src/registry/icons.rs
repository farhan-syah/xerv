//! Node icon constants.
//!
//! This module defines unique emoji icons for each node type in the standard library.
//! Icons are carefully selected to:
//! - Be visually distinct from each other
//! - Represent the node's purpose intuitively
//! - Render consistently across platforms

/// Icon for merge node (combines multiple inputs into one).
pub const ICON_MERGE: &str = "🔀";

/// Icon for switch node (conditional branching).
pub const ICON_SWITCH: &str = "↔️";

/// Icon for loop node (iteration and repetition).
pub const ICON_LOOP: &str = "🔁";

/// Icon for wait node (pause for human approval).
pub const ICON_WAIT: &str = "⏸️";

/// Icon for split node (fan-out to multiple outputs).
pub const ICON_SPLIT: &str = "📤";

/// Icon for map node (field transformation).
pub const ICON_MAP: &str = "🗺️";

/// Icon for concat node (string concatenation).
pub const ICON_CONCAT: &str = "➕";

/// Icon for aggregate node (statistical operations).
pub const ICON_AGGREGATE: &str = "📊";

/// Icon for JSON dynamic node (JSON manipulation).
pub const ICON_JSON_DYNAMIC: &str = "📋";

/// Icon for HTTP node (network requests).
pub const ICON_HTTP: &str = "🌐";

/// Icon for log node (logging output).
pub const ICON_LOG: &str = "📝";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_icons_are_unique() {
        let icons = vec![
            ICON_MERGE,
            ICON_SWITCH,
            ICON_LOOP,
            ICON_WAIT,
            ICON_SPLIT,
            ICON_MAP,
            ICON_CONCAT,
            ICON_AGGREGATE,
            ICON_JSON_DYNAMIC,
            ICON_HTTP,
            ICON_LOG,
        ];

        let unique: HashSet<_> = icons.iter().collect();
        assert_eq!(
            icons.len(),
            unique.len(),
            "All node icons must be unique. Found duplicates!"
        );
    }

    #[test]
    fn icons_are_non_empty() {
        let icons = vec![
            ICON_MERGE,
            ICON_SWITCH,
            ICON_LOOP,
            ICON_WAIT,
            ICON_SPLIT,
            ICON_MAP,
            ICON_CONCAT,
            ICON_AGGREGATE,
            ICON_JSON_DYNAMIC,
            ICON_HTTP,
            ICON_LOG,
        ];

        for icon in icons {
            assert!(!icon.is_empty(), "Icon must not be empty");
            assert!(
                icon.chars().count() >= 1,
                "Icon must have at least one character"
            );
        }
    }
}
