//! Selector expression parser.
//!
//! Parses expressions like `${fraud_check.score}` and `${pipeline.config.min_order_value}`.

use xerv_core::error::{Result, XervError};

/// A parsed selector expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// The full original expression (e.g., "${fraud_check.score}").
    pub raw: String,
    /// The root context (e.g., "fraud_check", "pipeline").
    pub root: String,
    /// The field path (e.g., ["score"] or ["config", "min_order_value"]).
    pub path: Vec<String>,
}

impl Selector {
    /// Create a new selector.
    pub fn new(root: impl Into<String>, path: Vec<String>) -> Self {
        let root = root.into();
        let raw = if path.is_empty() {
            format!("${{{}}}", root)
        } else {
            format!("${{{}.{}}}", root, path.join("."))
        };

        Self { raw, root, path }
    }

    /// Check if this selector references pipeline config.
    pub fn is_pipeline_config(&self) -> bool {
        self.root == "pipeline" && self.path.first().map(|s| s.as_str()) == Some("config")
    }

    /// Get the field path as a dot-separated string.
    pub fn field_path(&self) -> String {
        self.path.join(".")
    }

    /// Get the full path including root.
    pub fn full_path(&self) -> String {
        if self.path.is_empty() {
            self.root.clone()
        } else {
            format!("{}.{}", self.root, self.path.join("."))
        }
    }
}

/// Parser for selector expressions.
pub struct SelectorParser;

impl SelectorParser {
    /// Parse a string and extract all selector expressions.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let selectors = SelectorParser::parse_all("Score: ${fraud.score}, Config: ${pipeline.config.max}")?;
    /// assert_eq!(selectors.len(), 2);
    /// ```
    pub fn parse_all(input: &str) -> Result<Vec<Selector>> {
        let mut selectors = Vec::new();
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'

                // Find matching '}'
                let mut expr = String::new();
                let mut depth = 1;

                for ch in chars.by_ref() {
                    match ch {
                        '{' => {
                            depth += 1;
                            expr.push(ch);
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            expr.push(ch);
                        }
                        _ => expr.push(ch),
                    }
                }

                if depth != 0 {
                    return Err(XervError::SelectorSyntax {
                        selector: format!("${{{}...", expr),
                        cause: "Unclosed selector expression".to_string(),
                    });
                }

                let selector = Self::parse_expression(&expr)?;
                selectors.push(selector);
            }
        }

        Ok(selectors)
    }

    /// Parse a single selector expression (without the `${}` wrapper).
    pub fn parse_expression(expr: &str) -> Result<Selector> {
        let expr = expr.trim();

        if expr.is_empty() {
            return Err(XervError::SelectorSyntax {
                selector: String::new(),
                cause: "Empty selector expression".to_string(),
            });
        }

        // Split by '.'
        let parts: Vec<&str> = expr.split('.').collect();

        if parts.is_empty() {
            return Err(XervError::SelectorSyntax {
                selector: expr.to_string(),
                cause: "Invalid selector format".to_string(),
            });
        }

        // Validate identifiers
        for part in &parts {
            if !Self::is_valid_identifier(part) {
                return Err(XervError::SelectorSyntax {
                    selector: expr.to_string(),
                    cause: format!("Invalid identifier: {}", part),
                });
            }
        }

        let root = parts[0].to_string();
        let path: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();

        Ok(Selector {
            raw: format!("${{{}}}", expr),
            root,
            path,
        })
    }

    /// Check if a string is a valid identifier.
    fn is_valid_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }

        let mut chars = s.chars();
        let first = chars.next().unwrap();

        // First character must be letter or underscore
        if !first.is_ascii_alphabetic() && first != '_' {
            return false;
        }

        // Rest can be alphanumeric or underscore
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// Replace selectors in a string with their resolved values.
    ///
    /// Takes a resolver function that converts selectors to string values.
    pub fn interpolate<F>(input: &str, resolver: F) -> Result<String>
    where
        F: Fn(&Selector) -> Result<String>,
    {
        let mut result = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut in_selector = false;
        let mut selector_content = String::new();

        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'
                in_selector = true;
                selector_content.clear();
            } else if in_selector {
                if ch == '}' {
                    let selector = Self::parse_expression(&selector_content)?;
                    let value = resolver(&selector)?;
                    result.push_str(&value);
                    in_selector = false;
                } else {
                    selector_content.push(ch);
                }
            } else {
                result.push(ch);
            }
        }

        if in_selector {
            return Err(XervError::SelectorSyntax {
                selector: format!("${{{}...", selector_content),
                cause: "Unclosed selector expression".to_string(),
            });
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_selector() {
        let selector = SelectorParser::parse_expression("fraud_check.score").unwrap();
        assert_eq!(selector.root, "fraud_check");
        assert_eq!(selector.path, vec!["score"]);
    }

    #[test]
    fn parse_nested_selector() {
        let selector = SelectorParser::parse_expression("pipeline.config.min_order_value").unwrap();
        assert_eq!(selector.root, "pipeline");
        assert_eq!(selector.path, vec!["config", "min_order_value"]);
    }

    #[test]
    fn parse_multiple_selectors() {
        let input = "Score: ${fraud.score}, Config: ${pipeline.config.max}";
        let selectors = SelectorParser::parse_all(input).unwrap();
        assert_eq!(selectors.len(), 2);
        assert_eq!(selectors[0].root, "fraud");
        assert_eq!(selectors[1].root, "pipeline");
    }

    #[test]
    fn invalid_identifier() {
        let result = SelectorParser::parse_expression("123invalid");
        assert!(result.is_err());
    }

    #[test]
    fn empty_selector() {
        let result = SelectorParser::parse_expression("");
        assert!(result.is_err());
    }

    #[test]
    fn interpolate_string() {
        let input = "Fraud detected! Score: ${fraud.score}";
        let result = SelectorParser::interpolate(input, |selector| {
            if selector.root == "fraud" && selector.path == vec!["score"] {
                Ok("0.95".to_string())
            } else {
                Err(XervError::SelectorSyntax {
                    selector: selector.raw.clone(),
                    cause: "Unknown selector".to_string(),
                })
            }
        })
        .unwrap();

        assert_eq!(result, "Fraud detected! Score: 0.95");
    }

    #[test]
    fn is_pipeline_config() {
        let selector = SelectorParser::parse_expression("pipeline.config.min_order_value").unwrap();
        assert!(selector.is_pipeline_config());

        let selector = SelectorParser::parse_expression("fraud.score").unwrap();
        assert!(!selector.is_pipeline_config());
    }
}
