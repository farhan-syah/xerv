//! Schema compatibility analysis and change detection.
//!
//! Provides tools for comparing schemas, detecting changes, and
//! determining if migrations are required.

use super::migration::MigrationRegistry;
use super::version::ChangeKind;
use crate::traits::TypeInfo;
use std::sync::Arc;

/// Compatibility check result.
///
/// Contains details about whether two schemas are compatible,
/// what changes exist between them, and if migration is required.
#[derive(Debug, Clone)]
pub struct CompatibilityReport {
    /// Whether the schemas are directly compatible without requiring migration.
    pub compatible: bool,
    /// List of all changes detected between the schemas.
    pub changes: Vec<SchemaChange>,
    /// Whether a migration is required to use old data with the new schema.
    pub migration_required: bool,
    /// Whether a migration function is available for this transformation.
    pub migration_available: bool,
    /// Human-readable summary of the compatibility check result.
    pub summary: String,
}

impl CompatibilityReport {
    /// Create a compatibility report indicating full compatibility.
    pub fn compatible() -> Self {
        Self {
            compatible: true,
            changes: Vec::new(),
            migration_required: false,
            migration_available: true,
            summary: "Schemas are compatible".to_string(),
        }
    }

    /// Create a compatibility report indicating incompatibility.
    pub fn incompatible(changes: Vec<SchemaChange>, migration_available: bool) -> Self {
        let breaking_count = changes.iter().filter(|c| c.kind.is_breaking()).count();
        let summary = if migration_available {
            format!("{} breaking changes, migration available", breaking_count)
        } else {
            format!(
                "{} breaking changes, no migration available",
                breaking_count
            )
        };

        Self {
            compatible: false,
            changes,
            migration_required: true,
            migration_available,
            summary,
        }
    }

    /// Check if there are any breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        self.changes.iter().any(|c| c.kind.is_breaking())
    }

    /// Get all breaking changes.
    pub fn breaking_changes(&self) -> Vec<&SchemaChange> {
        self.changes
            .iter()
            .filter(|c| c.kind.is_breaking())
            .collect()
    }

    /// Get all non-breaking changes.
    pub fn non_breaking_changes(&self) -> Vec<&SchemaChange> {
        self.changes
            .iter()
            .filter(|c| !c.kind.is_breaking())
            .collect()
    }
}

/// Individual schema change.
#[derive(Debug, Clone)]
pub struct SchemaChange {
    /// The kind of change (e.g., AddOptionalField, RemoveField).
    pub kind: ChangeKind,
    /// The name of the field affected by this change.
    pub field: String,
    /// The previous type of the field, if applicable.
    pub old_type: Option<String>,
    /// The new type of the field, if applicable.
    pub new_type: Option<String>,
    /// Human-readable description of what changed and why.
    pub description: String,
}

impl SchemaChange {
    /// Create a new schema change.
    pub fn new(kind: ChangeKind, field: impl Into<String>) -> Self {
        let field = field.into();
        Self {
            description: format!("{}: {}", kind.description(), field),
            kind,
            field,
            old_type: None,
            new_type: None,
        }
    }

    /// Set the old type.
    pub fn with_old_type(mut self, old_type: impl Into<String>) -> Self {
        self.old_type = Some(old_type.into());
        self
    }

    /// Set the new type.
    pub fn with_new_type(mut self, new_type: impl Into<String>) -> Self {
        self.new_type = Some(new_type.into());
        self
    }

    /// Set a custom description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

impl std::fmt::Display for SchemaChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)?;
        if let (Some(old), Some(new)) = (&self.old_type, &self.new_type) {
            write!(f, " ({} -> {})", old, new)?;
        }
        Ok(())
    }
}

/// Compatibility matrix for schema pairs.
///
/// Provides methods for checking compatibility between schemas and
/// detecting changes.
pub struct CompatibilityMatrix {
    /// Migration registry for checking available migrations.
    migrations: Arc<MigrationRegistry>,
}

impl CompatibilityMatrix {
    /// Create a new compatibility matrix.
    pub fn new(migrations: Arc<MigrationRegistry>) -> Self {
        Self { migrations }
    }

    /// Check compatibility between two schemas.
    pub fn check(&self, from: &TypeInfo, to: &TypeInfo) -> CompatibilityReport {
        // Same schema hash = identical
        if from.hash != 0 && from.hash == to.hash {
            return CompatibilityReport::compatible();
        }

        // Detect changes
        let changes = self.detect_changes(from, to);

        if changes.is_empty() {
            return CompatibilityReport::compatible();
        }

        // Check for breaking changes
        let has_breaking = changes.iter().any(|c| c.kind.is_breaking());

        if !has_breaking {
            // Non-breaking changes only - compatible without migration
            return CompatibilityReport {
                compatible: true,
                changes,
                migration_required: false,
                migration_available: true,
                summary: "Compatible with non-breaking changes".to_string(),
            };
        }

        // Breaking changes - check if migration exists
        let migration_available = self.can_migrate(from.hash, to.hash);

        CompatibilityReport::incompatible(changes, migration_available)
    }

    /// Detect all changes between two schemas.
    pub fn detect_changes(&self, from: &TypeInfo, to: &TypeInfo) -> Vec<SchemaChange> {
        let mut changes = Vec::new();

        // Build field lookup for the source schema
        let from_fields: std::collections::HashMap<&str, _> =
            from.fields.iter().map(|f| (f.name.as_str(), f)).collect();

        // Build field lookup for the target schema
        let to_fields: std::collections::HashMap<&str, _> =
            to.fields.iter().map(|f| (f.name.as_str(), f)).collect();

        // Check for removed fields
        for (name, from_field) in &from_fields {
            if !to_fields.contains_key(name) {
                changes.push(
                    SchemaChange::new(ChangeKind::RemoveField, *name)
                        .with_old_type(&from_field.type_name),
                );
            }
        }

        // Check for added and modified fields
        for (name, to_field) in &to_fields {
            match from_fields.get(name) {
                None => {
                    // New field
                    let kind = if to_field.optional {
                        ChangeKind::AddOptionalField
                    } else {
                        ChangeKind::AddRequiredField
                    };
                    changes.push(SchemaChange::new(kind, *name).with_new_type(&to_field.type_name));
                }
                Some(from_field) => {
                    // Existing field - check for changes
                    if from_field.type_name != to_field.type_name {
                        changes.push(
                            SchemaChange::new(ChangeKind::ChangeFieldType, *name)
                                .with_old_type(&from_field.type_name)
                                .with_new_type(&to_field.type_name),
                        );
                    }

                    // Check optional -> required
                    if from_field.optional && !to_field.optional {
                        changes.push(SchemaChange::new(ChangeKind::MakeRequired, *name));
                    }

                    // Check required -> optional
                    if !from_field.optional && to_field.optional {
                        changes.push(SchemaChange::new(ChangeKind::MakeOptional, *name));
                    }
                }
            }
        }

        changes
    }

    /// Check if runtime migration is possible.
    pub fn can_migrate(&self, from_hash: u64, to_hash: u64) -> bool {
        self.migrations.has_path(from_hash, to_hash)
    }

    /// Detect potential field renames between schemas.
    ///
    /// This is a heuristic: fields with the same type but different names
    /// might be renames. Returns pairs of (old_name, new_name).
    pub fn detect_potential_renames(
        &self,
        from: &TypeInfo,
        to: &TypeInfo,
    ) -> Vec<(String, String)> {
        let mut potential_renames = Vec::new();

        // Find removed fields
        let removed: Vec<_> = from
            .fields
            .iter()
            .filter(|f| !to.fields.iter().any(|t| t.name == f.name))
            .collect();

        // Find added fields
        let added: Vec<_> = to
            .fields
            .iter()
            .filter(|f| !from.fields.iter().any(|t| t.name == f.name))
            .collect();

        // Match by type
        for removed_field in &removed {
            for added_field in &added {
                if removed_field.type_name == added_field.type_name
                    && removed_field.optional == added_field.optional
                {
                    potential_renames.push((removed_field.name.clone(), added_field.name.clone()));
                }
            }
        }

        potential_renames
    }
}

impl Default for CompatibilityMatrix {
    fn default() -> Self {
        Self::new(Arc::new(MigrationRegistry::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::FieldInfo;

    fn create_v1_schema() -> TypeInfo {
        TypeInfo::new("Order", 1).with_hash(100).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("amount", "f64"),
            FieldInfo::new("status", "String"),
        ])
    }

    fn create_v2_schema_compatible() -> TypeInfo {
        TypeInfo::new("Order", 2).with_hash(200).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("amount", "f64"),
            FieldInfo::new("status", "String"),
            FieldInfo::new("notes", "String").optional(),
        ])
    }

    fn create_v2_schema_breaking() -> TypeInfo {
        TypeInfo::new("Order", 2).with_hash(200).with_fields(vec![
            FieldInfo::new("id", "String"),
            FieldInfo::new("total", "f64"),  // renamed from amount
            FieldInfo::new("status", "i32"), // type changed
        ])
    }

    #[test]
    fn identical_schemas() {
        let matrix = CompatibilityMatrix::default();
        let v1 = create_v1_schema();

        let report = matrix.check(&v1, &v1);
        assert!(report.compatible);
        assert!(report.changes.is_empty());
    }

    #[test]
    fn compatible_with_optional_field() {
        let matrix = CompatibilityMatrix::default();
        let v1 = create_v1_schema();
        let v2 = create_v2_schema_compatible();

        let report = matrix.check(&v1, &v2);
        assert!(report.compatible);
        assert!(!report.migration_required);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(report.changes[0].kind, ChangeKind::AddOptionalField);
    }

    #[test]
    fn breaking_changes() {
        let matrix = CompatibilityMatrix::default();
        let v1 = create_v1_schema();
        let v2 = create_v2_schema_breaking();

        let report = matrix.check(&v1, &v2);
        assert!(!report.compatible);
        assert!(report.migration_required);
        assert!(report.has_breaking_changes());

        // Should detect: removed 'amount', added 'total', changed 'status' type
        let breaking = report.breaking_changes();
        assert!(!breaking.is_empty());
    }

    #[test]
    fn detect_changes_removed_field() {
        let matrix = CompatibilityMatrix::default();

        let v1 = TypeInfo::new("Test", 1)
            .with_fields(vec![FieldInfo::new("a", "i32"), FieldInfo::new("b", "i32")]);

        let v2 = TypeInfo::new("Test", 2).with_fields(vec![FieldInfo::new("a", "i32")]);

        let changes = matrix.detect_changes(&v1, &v2);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::RemoveField);
        assert_eq!(changes[0].field, "b");
    }

    #[test]
    fn detect_changes_type_change() {
        let matrix = CompatibilityMatrix::default();

        let v1 = TypeInfo::new("Test", 1).with_fields(vec![FieldInfo::new("value", "i32")]);

        let v2 = TypeInfo::new("Test", 2).with_fields(vec![FieldInfo::new("value", "f64")]);

        let changes = matrix.detect_changes(&v1, &v2);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::ChangeFieldType);
        assert_eq!(changes[0].old_type, Some("i32".to_string()));
        assert_eq!(changes[0].new_type, Some("f64".to_string()));
    }

    #[test]
    fn detect_potential_renames() {
        let matrix = CompatibilityMatrix::default();

        let v1 = TypeInfo::new("Test", 1).with_fields(vec![
            FieldInfo::new("old_name", "String"),
            FieldInfo::new("other", "i32"),
        ]);

        let v2 = TypeInfo::new("Test", 2).with_fields(vec![
            FieldInfo::new("new_name", "String"),
            FieldInfo::new("other", "i32"),
        ]);

        let renames = matrix.detect_potential_renames(&v1, &v2);
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0], ("old_name".to_string(), "new_name".to_string()));
    }

    #[test]
    fn with_migration_available() {
        let migrations = Arc::new(MigrationRegistry::new());

        // Register a migration
        use super::super::migration::Migration;
        migrations
            .register(Migration::new(
                "Order@v1",
                100,
                "Order@v2",
                200,
                |_arena, offset| Ok(offset),
            ))
            .unwrap();

        let matrix = CompatibilityMatrix::new(migrations);
        let v1 = create_v1_schema();
        let v2 = create_v2_schema_breaking();

        let report = matrix.check(&v1, &v2);
        assert!(!report.compatible);
        assert!(report.migration_available);
    }
}
