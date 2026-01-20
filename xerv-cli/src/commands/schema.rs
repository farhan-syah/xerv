//! Schema management CLI commands.
//!
//! Provides commands for listing, inspecting, and validating schemas
//! and their migrations.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use xerv_core::schema::{
    CompatibilityMatrix, MigrationRegistry, SchemaVersion, VersionedSchemaRegistry,
};

/// List all registered schemas.
pub async fn list(registry_path: Option<&str>) -> Result<()> {
    let registry = load_registry(registry_path)?;

    let families = registry.families();
    if families.is_empty() {
        println!("No schemas registered.");
        return Ok(());
    }

    println!("Registered Schemas:");
    println!("{:-<60}", "");

    for family in &families {
        let versions = registry.get_versions(family);
        let version_str: Vec<String> = versions.iter().map(|v| v.to_string()).collect();

        println!("  {}", family);
        println!("    Versions: {}", version_str.join(", "));

        if let Some(latest) = registry.get_latest(family) {
            println!("    Latest: {} (hash: {})", latest.version, latest.hash);
            println!("    Fields: {}", latest.fields.len());
        }
        println!();
    }

    println!(
        "Total: {} schema families, {} versions",
        families.len(),
        registry.len()
    );

    Ok(())
}

/// Show details of a specific schema.
pub async fn show(name: &str, registry_path: Option<&str>) -> Result<()> {
    let registry = load_registry(registry_path)?;

    // Try to find by full name or short name
    let schemas: Vec<_> = if name.contains('@') {
        // Full name like "Order@v1"
        registry.get(name).into_iter().collect()
    } else {
        // Short name - show all versions
        registry
            .get_versions(name)
            .into_iter()
            .filter_map(|v| registry.get_version(name, v))
            .collect()
    };

    if schemas.is_empty() {
        println!("Schema '{}' not found.", name);
        return Ok(());
    }

    for schema in schemas {
        println!("Schema: {}", schema.name);
        println!("{:-<60}", "");
        println!("  Short Name: {}", schema.short_name);
        println!("  Version: {}", schema.version);
        println!("  Hash: {}", schema.hash);
        println!("  Size: {} bytes", schema.size);
        println!("  Alignment: {} bytes", schema.alignment);
        println!("  Stable Layout: {}", schema.stable_layout);
        println!();
        println!("  Fields:");
        for field in &schema.fields {
            let optional = if field.optional { " (optional)" } else { "" };
            println!(
                "    - {} : {} @ offset {}, {} bytes{}",
                field.name, field.type_name, field.offset, field.size, optional
            );
        }
        println!();
    }

    Ok(())
}

/// Check compatibility between two schema versions.
pub async fn check(from: &str, to: &str, registry_path: Option<&str>) -> Result<()> {
    let registry = load_registry(registry_path)?;
    let migrations = Arc::new(MigrationRegistry::new());
    let matrix = CompatibilityMatrix::new(migrations);

    let from_schema = registry
        .get(from)
        .ok_or_else(|| anyhow::anyhow!("Source schema '{}' not found", from))?;

    let to_schema = registry
        .get(to)
        .ok_or_else(|| anyhow::anyhow!("Target schema '{}' not found", to))?;

    let report = matrix.check(&from_schema, &to_schema);

    println!("Compatibility Check: {} → {}", from, to);
    println!("{:-<60}", "");
    println!();

    if report.compatible {
        println!("✓ Schemas are compatible");
    } else {
        println!("✗ Schemas are incompatible");
    }

    println!("  Migration Required: {}", report.migration_required);
    println!("  Migration Available: {}", report.migration_available);
    println!();

    if !report.changes.is_empty() {
        println!("Changes:");
        for change in &report.changes {
            let breaking = if change.kind.is_breaking() {
                " [BREAKING]"
            } else {
                ""
            };
            println!("  - {}{}", change, breaking);
        }
    } else {
        println!("No changes detected.");
    }

    println!();
    println!("Summary: {}", report.summary);

    Ok(())
}

/// Validate a migration path between schema versions.
pub async fn validate_migration(from: &str, to: &str, registry_path: Option<&str>) -> Result<()> {
    let registry = load_registry(registry_path)?;
    let migrations = MigrationRegistry::new();

    let from_schema = registry
        .get(from)
        .ok_or_else(|| anyhow::anyhow!("Source schema '{}' not found", from))?;

    let to_schema = registry
        .get(to)
        .ok_or_else(|| anyhow::anyhow!("Target schema '{}' not found", to))?;

    println!("Migration Path Validation: {} → {}", from, to);
    println!("{:-<60}", "");
    println!();

    if let Some(path) = migrations.find_path(from_schema.hash, to_schema.hash) {
        if path.is_empty() {
            println!("✓ No migration needed (same schema)");
        } else {
            println!("✓ Migration path found ({} steps):", path.len());
            for (i, migration) in path.iter().enumerate() {
                println!(
                    "  {}. {} → {}",
                    i + 1,
                    migration.from_schema,
                    migration.to_schema
                );
                if !migration.description.is_empty() {
                    println!("     {}", migration.description);
                }
            }
        }
    } else {
        println!("✗ No migration path available");
        println!();
        println!("Tip: Register a migration function using #[xerv::migration]");
    }

    Ok(())
}

/// Export the schema registry to a file.
pub async fn export(path: &str, registry_path: Option<&str>) -> Result<()> {
    let registry = if let Some(rp) = registry_path {
        VersionedSchemaRegistry::load(rp)?
    } else {
        // Create empty registry if no path specified
        VersionedSchemaRegistry::new()
    };

    // Create a new registry with the export path for persistence
    let export_registry = VersionedSchemaRegistry::with_persistence(path);

    // Copy all schemas (we need to re-register them)
    for name in registry.names() {
        if let Some(schema) = registry.get(&name) {
            export_registry.register(schema);
        }
    }

    export_registry.persist()?;

    println!("Exported {} schemas to {}", registry.len(), path);

    Ok(())
}

/// Import schemas from a file.
pub async fn import(path: &str) -> Result<()> {
    let registry = VersionedSchemaRegistry::load(path)?;

    println!("Imported {} schemas from {}", registry.len(), path);
    println!();

    // Show what was imported
    for family in registry.families() {
        let versions = registry.get_versions(&family);
        println!("  {} ({} versions)", family, versions.len());
    }

    Ok(())
}

/// Print schema version information.
pub async fn version_info(version_str: &str) -> Result<()> {
    match SchemaVersion::parse(version_str) {
        Some(version) => {
            println!("Parsed Version: {}", version);
            println!("  Major: {}", version.major);
            println!("  Minor: {}", version.minor);
            println!("  Short String: {}", version.to_short_string());
            println!("  Next Minor: {}", version.next_minor());
            println!("  Next Major: {}", version.next_major());
        }
        None => {
            println!("Invalid version format: {}", version_str);
            println!();
            println!("Valid formats:");
            println!("  v1, v1.0, v1.2");
            println!("  1, 1.0, 1.2");
        }
    }

    Ok(())
}

/// Load the schema registry from a path or return an empty one.
fn load_registry(path: Option<&str>) -> Result<VersionedSchemaRegistry> {
    match path {
        Some(p) if Path::new(p).exists() => Ok(VersionedSchemaRegistry::load(p)?),
        _ => Ok(VersionedSchemaRegistry::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        assert!(SchemaVersion::parse("v1").is_some());
        assert!(SchemaVersion::parse("v1.2").is_some());
        assert!(SchemaVersion::parse("invalid").is_none());
    }
}
