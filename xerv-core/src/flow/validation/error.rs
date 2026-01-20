//! Validation error types.

/// A validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// The type of error.
    pub kind: ValidationErrorKind,
    /// The location in the flow (e.g., "nodes.fraud_check").
    pub location: String,
    /// Human-readable error message.
    pub message: String,
}

/// Types of validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationErrorKind {
    /// Missing required field.
    MissingField,
    /// Invalid value for a field.
    InvalidValue,
    /// Duplicate identifier.
    DuplicateId,
    /// Reference to non-existent node.
    InvalidReference,
    /// Invalid trigger type.
    InvalidTriggerType,
    /// Invalid node type.
    InvalidNodeType,
    /// Cycle detected (without proper loop declaration).
    CycleDetected,
    /// Unreachable node.
    UnreachableNode,
    /// Invalid selector syntax.
    InvalidSelector,
    /// Validation limit exceeded (size, depth, count).
    LimitExceeded,
    /// Security violation (disallowed path, etc.).
    SecurityViolation,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.kind, self.location, self.message)
    }
}

impl std::fmt::Display for ValidationErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::MissingField => "MISSING_FIELD",
            Self::InvalidValue => "INVALID_VALUE",
            Self::DuplicateId => "DUPLICATE_ID",
            Self::InvalidReference => "INVALID_REFERENCE",
            Self::InvalidTriggerType => "INVALID_TRIGGER_TYPE",
            Self::InvalidNodeType => "INVALID_NODE_TYPE",
            Self::CycleDetected => "CYCLE_DETECTED",
            Self::UnreachableNode => "UNREACHABLE_NODE",
            Self::InvalidSelector => "INVALID_SELECTOR",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::SecurityViolation => "SECURITY_VIOLATION",
        };
        write!(f, "{}", s)
    }
}

impl ValidationError {
    /// Create a new validation error.
    pub fn new(
        kind: ValidationErrorKind,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            location: location.into(),
            message: message.into(),
        }
    }

    /// Create a missing field error.
    pub fn missing_field(location: impl Into<String>, field: &str) -> Self {
        Self::new(
            ValidationErrorKind::MissingField,
            location,
            format!("missing required field '{}'", field),
        )
    }

    /// Create an invalid value error.
    pub fn invalid_value(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ValidationErrorKind::InvalidValue, location, message)
    }

    /// Create a duplicate ID error.
    pub fn duplicate_id(location: impl Into<String>, id: &str) -> Self {
        Self::new(
            ValidationErrorKind::DuplicateId,
            location,
            format!("duplicate identifier '{}'", id),
        )
    }

    /// Create an invalid reference error.
    pub fn invalid_reference(location: impl Into<String>, reference: &str) -> Self {
        Self::new(
            ValidationErrorKind::InvalidReference,
            location,
            format!("reference to non-existent node '{}'", reference),
        )
    }
}
