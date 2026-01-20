//! Schema migration registry and transform logic.
//!
//! Provides infrastructure for registering and applying schema migrations.
//! Migrations transform data from one schema version to another.

use super::version::SchemaVersion;
use crate::arena::Arena;
use crate::error::{Result, XervError};
use crate::types::ArenaOffset;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Migration function signature.
///
/// A migration function reads data at a source offset, transforms it,
/// and writes the result to the arena, returning the new offset.
///
/// # Arguments
///
/// * `arena` - The arena containing the source data
/// * `src_offset` - The offset of the source data in the arena
///
/// # Returns
///
/// The offset of the transformed data in the arena, or an error.
pub type MigrationFn = Arc<dyn Fn(&Arena, ArenaOffset) -> Result<ArenaOffset> + Send + Sync>;

/// A registered migration between schema versions.
#[derive(Clone)]
pub struct Migration {
    /// Source schema name with version (e.g., "OrderInput@v1").
    pub from_schema: String,
    /// Source schema hash.
    pub from_hash: u64,
    /// Target schema name with version (e.g., "OrderInput@v2").
    pub to_schema: String,
    /// Target schema hash.
    pub to_hash: u64,
    /// Source version.
    pub from_version: SchemaVersion,
    /// Target version.
    pub to_version: SchemaVersion,
    /// The transform function.
    pub transform: MigrationFn,
    /// Human-readable description of what this migration does.
    pub description: String,
}

impl Migration {
    /// Create a new migration.
    pub fn new<F>(
        from_schema: impl Into<String>,
        from_hash: u64,
        to_schema: impl Into<String>,
        to_hash: u64,
        transform: F,
    ) -> Self
    where
        F: Fn(&Arena, ArenaOffset) -> Result<ArenaOffset> + Send + Sync + 'static,
    {
        let from = from_schema.into();
        let to = to_schema.into();

        // Extract versions from schema names
        let from_version = extract_version(&from).unwrap_or_default();
        let to_version = extract_version(&to).unwrap_or_default();

        Self {
            from_schema: from,
            from_hash,
            to_schema: to,
            to_hash,
            from_version,
            to_version,
            transform: Arc::new(transform),
            description: String::new(),
        }
    }

    /// Add a description to this migration.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Apply this migration to data at the given offset.
    pub fn apply(&self, arena: &Arena, offset: ArenaOffset) -> Result<ArenaOffset> {
        (self.transform)(arena, offset)
    }
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("from_schema", &self.from_schema)
            .field("from_hash", &self.from_hash)
            .field("to_schema", &self.to_schema)
            .field("to_hash", &self.to_hash)
            .field("description", &self.description)
            .finish()
    }
}

/// Graph for computing migration paths.
///
/// Uses BFS to find the shortest migration path between schemas.
struct MigrationGraph {
    /// Edges: from_hash -> [(to_hash, migration_index)]
    edges: HashMap<u64, Vec<(u64, usize)>>,
}

impl MigrationGraph {
    fn new() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }

    fn add_edge(&mut self, from_hash: u64, to_hash: u64, migration_index: usize) {
        self.edges
            .entry(from_hash)
            .or_default()
            .push((to_hash, migration_index));
    }

    fn remove_edges_for(&mut self, from_hash: u64, to_hash: u64) {
        if let Some(edges) = self.edges.get_mut(&from_hash) {
            edges.retain(|(hash, _)| *hash != to_hash);
        }
    }

    /// Find shortest path from source to target using BFS.
    ///
    /// Returns the migration indices in order.
    fn shortest_path(&self, from_hash: u64, to_hash: u64) -> Option<Vec<usize>> {
        if from_hash == to_hash {
            return Some(Vec::new());
        }

        // BFS with parent tracking
        let mut visited: HashMap<u64, (u64, usize)> = HashMap::new();
        let mut queue = std::collections::VecDeque::new();

        queue.push_back(from_hash);
        visited.insert(from_hash, (0, usize::MAX)); // Sentinel for start

        while let Some(current) = queue.pop_front() {
            if let Some(neighbors) = self.edges.get(&current) {
                for &(next_hash, migration_idx) in neighbors {
                    if next_hash == to_hash {
                        // Found the target - reconstruct path
                        let mut path = vec![migration_idx];
                        let mut node = current;

                        while let Some(&(parent, idx)) = visited.get(&node) {
                            if idx == usize::MAX {
                                break; // Reached start
                            }
                            path.push(idx);
                            node = parent;
                        }

                        path.reverse();
                        return Some(path);
                    }

                    if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(next_hash) {
                        e.insert((current, migration_idx));
                        queue.push_back(next_hash);
                    }
                }
            }
        }

        None
    }
}

/// Registry of schema migrations.
///
/// Stores migrations between schema versions and provides path-finding
/// for multi-hop migrations (e.g., v1 → v2 → v3).
pub struct MigrationRegistry {
    /// All registered migrations.
    migrations: RwLock<Vec<Migration>>,
    /// Direct migrations keyed by (from_hash, to_hash).
    direct: RwLock<HashMap<(u64, u64), usize>>,
    /// Graph for path finding.
    graph: RwLock<MigrationGraph>,
}

impl MigrationRegistry {
    /// Create a new empty migration registry.
    pub fn new() -> Self {
        Self {
            migrations: RwLock::new(Vec::new()),
            direct: RwLock::new(HashMap::new()),
            graph: RwLock::new(MigrationGraph::new()),
        }
    }

    /// Register a migration.
    ///
    /// If a migration between the same schemas already exists, it is replaced.
    pub fn register(&self, migration: Migration) -> Result<()> {
        let key = (migration.from_hash, migration.to_hash);

        let mut migrations = self.migrations.write();
        let mut direct = self.direct.write();
        let mut graph = self.graph.write();

        // Check if migration already exists
        if let Some(&existing_idx) = direct.get(&key) {
            // Replace existing migration
            migrations[existing_idx] = migration;
        } else {
            // Add new migration
            let idx = migrations.len();
            migrations.push(migration);
            direct.insert(key, idx);
            graph.add_edge(key.0, key.1, idx);
        }

        Ok(())
    }

    /// Unregister a migration.
    pub fn unregister(&self, from_hash: u64, to_hash: u64) -> Option<Migration> {
        let key = (from_hash, to_hash);

        let migrations = self.migrations.read();
        let mut direct = self.direct.write();
        let mut graph = self.graph.write();

        if let Some(idx) = direct.remove(&key) {
            graph.remove_edges_for(from_hash, to_hash);
            // We don't actually remove from the vec to keep indices stable
            // Just return a clone
            Some(migrations[idx].clone())
        } else {
            None
        }
    }

    /// Find a direct migration from one schema to another.
    pub fn find_direct(&self, from_hash: u64, to_hash: u64) -> Option<Migration> {
        let direct = self.direct.read();
        let migrations = self.migrations.read();

        direct
            .get(&(from_hash, to_hash))
            .map(|&idx| migrations[idx].clone())
    }

    /// Find the migration path from one schema to another.
    ///
    /// Returns the migrations in order, or None if no path exists.
    pub fn find_path(&self, from_hash: u64, to_hash: u64) -> Option<Vec<Migration>> {
        if from_hash == to_hash {
            return Some(Vec::new());
        }

        let graph = self.graph.read();
        let migrations = self.migrations.read();

        graph.shortest_path(from_hash, to_hash).map(|indices| {
            indices
                .into_iter()
                .map(|idx| migrations[idx].clone())
                .collect()
        })
    }

    /// Check if a migration path exists.
    pub fn has_path(&self, from_hash: u64, to_hash: u64) -> bool {
        if from_hash == to_hash {
            return true;
        }

        let graph = self.graph.read();
        graph.shortest_path(from_hash, to_hash).is_some()
    }

    /// Apply migration to arena data.
    ///
    /// Finds the migration path and applies each migration in sequence.
    pub fn migrate(
        &self,
        arena: &Arena,
        offset: ArenaOffset,
        from_hash: u64,
        to_hash: u64,
    ) -> Result<ArenaOffset> {
        if from_hash == to_hash {
            return Ok(offset);
        }

        let path =
            self.find_path(from_hash, to_hash)
                .ok_or_else(|| XervError::SchemaValidation {
                    schema: format!("hash:{}", from_hash),
                    cause: format!("No migration path to hash:{}", to_hash),
                })?;

        let mut current_offset = offset;
        for migration in path {
            current_offset = migration.apply(arena, current_offset)?;
        }

        Ok(current_offset)
    }

    /// Get all registered migrations.
    pub fn list(&self) -> Vec<Migration> {
        self.migrations.read().clone()
    }

    /// Get the number of registered migrations.
    pub fn len(&self) -> usize {
        self.migrations.read().len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.migrations.read().is_empty()
    }

    /// Get migrations from a specific schema.
    pub fn migrations_from(&self, from_hash: u64) -> Vec<Migration> {
        let migrations = self.migrations.read();
        let graph = self.graph.read();

        graph
            .edges
            .get(&from_hash)
            .map(|edges| {
                edges
                    .iter()
                    .map(|&(_, idx)| migrations[idx].clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get migrations to a specific schema.
    pub fn migrations_to(&self, to_hash: u64) -> Vec<Migration> {
        let migrations = self.migrations.read();
        let direct = self.direct.read();

        direct
            .iter()
            .filter(|&(&(_, to), _)| to == to_hash)
            .map(|(_, &idx)| migrations[idx].clone())
            .collect()
    }
}

impl Default for MigrationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract version from a schema name like "OrderInput@v1".
fn extract_version(schema_name: &str) -> Option<SchemaVersion> {
    let version_part = schema_name.rsplit('@').next()?;
    SchemaVersion::parse(version_part)
}

/// Compose multiple migration functions into one.
///
/// This is useful when you need to apply multiple migrations in sequence
/// but want to treat them as a single operation.
pub fn compose_migrations(migrations: Vec<Migration>) -> MigrationFn {
    Arc::new(move |arena, offset| {
        let mut current = offset;
        for migration in &migrations {
            current = migration.apply(arena, current)?;
        }
        Ok(current)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_migration(from_hash: u64, to_hash: u64) -> Migration {
        Migration::new(
            format!("Test@v{}", from_hash),
            from_hash,
            format!("Test@v{}", to_hash),
            to_hash,
            move |_arena, offset| {
                // Mock: just return the same offset
                Ok(offset)
            },
        )
        .with_description(format!("Migrate {} to {}", from_hash, to_hash))
    }

    #[test]
    fn register_and_find_direct() {
        let registry = MigrationRegistry::new();

        let migration = mock_migration(100, 200);
        registry.register(migration).unwrap();

        assert!(registry.find_direct(100, 200).is_some());
        assert!(registry.find_direct(200, 100).is_none());
        assert!(registry.find_direct(100, 300).is_none());
    }

    #[test]
    fn find_path_direct() {
        let registry = MigrationRegistry::new();

        registry.register(mock_migration(100, 200)).unwrap();

        let path = registry.find_path(100, 200).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].from_hash, 100);
        assert_eq!(path[0].to_hash, 200);
    }

    #[test]
    fn find_path_multi_hop() {
        let registry = MigrationRegistry::new();

        registry.register(mock_migration(100, 200)).unwrap();
        registry.register(mock_migration(200, 300)).unwrap();
        registry.register(mock_migration(300, 400)).unwrap();

        // Direct path
        let path = registry.find_path(100, 200).unwrap();
        assert_eq!(path.len(), 1);

        // Multi-hop path
        let path = registry.find_path(100, 400).unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].from_hash, 100);
        assert_eq!(path[1].from_hash, 200);
        assert_eq!(path[2].from_hash, 300);
    }

    #[test]
    fn find_path_shortest() {
        let registry = MigrationRegistry::new();

        // Long path: 100 -> 200 -> 300 -> 400
        registry.register(mock_migration(100, 200)).unwrap();
        registry.register(mock_migration(200, 300)).unwrap();
        registry.register(mock_migration(300, 400)).unwrap();

        // Short path: 100 -> 400
        registry.register(mock_migration(100, 400)).unwrap();

        // Should find the shortest path
        let path = registry.find_path(100, 400).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].to_hash, 400);
    }

    #[test]
    fn no_path() {
        let registry = MigrationRegistry::new();

        registry.register(mock_migration(100, 200)).unwrap();
        registry.register(mock_migration(300, 400)).unwrap();

        // No path from 100 to 400
        assert!(registry.find_path(100, 400).is_none());
        assert!(!registry.has_path(100, 400));
    }

    #[test]
    fn same_version_empty_path() {
        let registry = MigrationRegistry::new();

        let path = registry.find_path(100, 100).unwrap();
        assert!(path.is_empty());
        assert!(registry.has_path(100, 100));
    }

    #[test]
    fn replace_migration() {
        let registry = MigrationRegistry::new();

        let m1 = mock_migration(100, 200).with_description("First");
        let m2 = mock_migration(100, 200).with_description("Second");

        registry.register(m1).unwrap();
        assert_eq!(registry.len(), 1);

        registry.register(m2).unwrap();
        assert_eq!(registry.len(), 1);

        let found = registry.find_direct(100, 200).unwrap();
        assert_eq!(found.description, "Second");
    }

    #[test]
    fn migrations_from_and_to() {
        let registry = MigrationRegistry::new();

        registry.register(mock_migration(100, 200)).unwrap();
        registry.register(mock_migration(100, 300)).unwrap();
        registry.register(mock_migration(200, 300)).unwrap();

        let from_100 = registry.migrations_from(100);
        assert_eq!(from_100.len(), 2);

        let to_300 = registry.migrations_to(300);
        assert_eq!(to_300.len(), 2);
    }

    #[test]
    fn extract_version_from_name() {
        assert_eq!(
            extract_version("OrderInput@v1"),
            Some(SchemaVersion::new(1, 0))
        );
        assert_eq!(
            extract_version("OrderInput@v2.1"),
            Some(SchemaVersion::new(2, 1))
        );
        assert_eq!(extract_version("OrderInput"), None);
    }
}
