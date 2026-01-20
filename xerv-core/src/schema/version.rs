//! Schema version model and compatibility rules.
//!
//! Provides semantic versioning for schemas with major.minor version numbers.
//! Breaking changes require major version bumps, non-breaking changes use minor.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Semantic version for schemas.
///
/// Uses major.minor versioning:
/// - Major version changes indicate breaking changes
/// - Minor version changes indicate backward-compatible changes
///
/// # Example
///
/// ```ignore
/// let v1 = SchemaVersion::new(1, 0);
/// let v1_1 = SchemaVersion::new(1, 1);
/// let v2 = SchemaVersion::new(2, 0);
///
/// assert!(v1.is_compatible_with(&v1_1)); // Same major = compatible
/// assert!(!v1.is_compatible_with(&v2));  // Different major = breaking
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchemaVersion {
    /// Major version (breaking changes).
    pub major: u16,
    /// Minor version (backward-compatible changes).
    pub minor: u16,
}

impl SchemaVersion {
    /// Create a new schema version.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Parse a version string (e.g., "v1", "v1.2", "1.0").
    ///
    /// Supports formats:
    /// - "v1" → (1, 0)
    /// - "v1.2" → (1, 2)
    /// - "1.0" → (1, 0)
    /// - "1" → (1, 0)
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').unwrap_or(s);

        if let Some((major_str, minor_str)) = s.split_once('.') {
            let major = major_str.parse().ok()?;
            let minor = minor_str.parse().ok()?;
            Some(Self::new(major, minor))
        } else {
            let major = s.parse().ok()?;
            Some(Self::new(major, 0))
        }
    }

    /// Check if this version is compatible with another version.
    ///
    /// Two versions are compatible if they have the same major version,
    /// and this version's minor is >= the other's minor.
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major && self.minor >= other.minor
    }

    /// Check if this version is a direct successor to another.
    pub fn is_successor_of(&self, other: &Self) -> bool {
        (self.major == other.major && self.minor == other.minor + 1)
            || (self.major == other.major + 1 && self.minor == 0)
    }

    /// Get the next minor version.
    pub fn next_minor(&self) -> Self {
        Self::new(self.major, self.minor + 1)
    }

    /// Get the next major version.
    pub fn next_major(&self) -> Self {
        Self::new(self.major + 1, 0)
    }

    /// Format as a version string (e.g., "v1.0").
    pub fn to_version_string(&self) -> String {
        format!("v{}.{}", self.major, self.minor)
    }

    /// Format as a short version string (e.g., "v1" if minor is 0).
    pub fn to_short_string(&self) -> String {
        if self.minor == 0 {
            format!("v{}", self.major)
        } else {
            self.to_version_string()
        }
    }
}

impl Default for SchemaVersion {
    fn default() -> Self {
        Self::new(1, 0)
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}", self.major, self.minor)
    }
}

impl FromStr for SchemaVersion {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or("Invalid version format")
    }
}

impl PartialOrd for SchemaVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchemaVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => self.minor.cmp(&other.minor),
            ord => ord,
        }
    }
}

/// Version range for compatibility declarations.
///
/// Specifies a range of compatible versions, useful for declaring
/// which schema versions a migration or node supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange {
    /// Minimum version (inclusive).
    pub min: SchemaVersion,
    /// Maximum version (inclusive, if specified).
    pub max: Option<SchemaVersion>,
}

impl VersionRange {
    /// Create a range starting from a specific version with no upper bound.
    pub fn from(version: SchemaVersion) -> Self {
        Self {
            min: version,
            max: None,
        }
    }

    /// Create a range between two versions (inclusive).
    pub fn between(min: SchemaVersion, max: SchemaVersion) -> Self {
        Self {
            min,
            max: Some(max),
        }
    }

    /// Create an exact version range (single version).
    pub fn exact(version: SchemaVersion) -> Self {
        Self {
            min: version,
            max: Some(version),
        }
    }

    /// Check if a version falls within this range.
    pub fn contains(&self, version: SchemaVersion) -> bool {
        if version < self.min {
            return false;
        }
        match self.max {
            Some(max) => version <= max,
            None => true,
        }
    }

    /// Check if this range overlaps with another.
    pub fn overlaps(&self, other: &VersionRange) -> bool {
        // Check if ranges have any intersection
        let self_max = self.max.unwrap_or(SchemaVersion::new(u16::MAX, u16::MAX));
        let other_max = other.max.unwrap_or(SchemaVersion::new(u16::MAX, u16::MAX));

        self.min <= other_max && other.min <= self_max
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.max {
            Some(max) if max == self.min => write!(f, "{}", self.min),
            Some(max) => write!(f, "{}-{}", self.min, max),
            None => write!(f, "{}+", self.min),
        }
    }
}

/// Type of change between schema versions.
///
/// Used to classify schema modifications and determine if a
/// migration is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// New optional field added (backward compatible).
    AddOptionalField,
    /// New required field added (breaking without default).
    AddRequiredField,
    /// Field removed (breaking).
    RemoveField,
    /// Field type changed (breaking).
    ChangeFieldType,
    /// Field renamed (breaking without migration).
    RenameField,
    /// Field marked as deprecated (non-breaking).
    DeprecateField,
    /// Field default value changed (non-breaking).
    ChangeDefault,
    /// Field made optional (non-breaking).
    MakeOptional,
    /// Field made required (breaking).
    MakeRequired,
}

impl ChangeKind {
    /// Check if this change kind is breaking.
    ///
    /// Breaking changes require a major version bump and typically
    /// need a migration function to transform old data.
    pub fn is_breaking(&self) -> bool {
        matches!(
            self,
            Self::AddRequiredField
                | Self::RemoveField
                | Self::ChangeFieldType
                | Self::RenameField
                | Self::MakeRequired
        )
    }

    /// Get a human-readable description of this change kind.
    pub fn description(&self) -> &'static str {
        match self {
            Self::AddOptionalField => "Added optional field",
            Self::AddRequiredField => "Added required field",
            Self::RemoveField => "Removed field",
            Self::ChangeFieldType => "Changed field type",
            Self::RenameField => "Renamed field",
            Self::DeprecateField => "Deprecated field",
            Self::ChangeDefault => "Changed default value",
            Self::MakeOptional => "Made field optional",
            Self::MakeRequired => "Made field required",
        }
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_creation() {
        let v = SchemaVersion::new(1, 2);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
    }

    #[test]
    fn version_parsing() {
        assert_eq!(SchemaVersion::parse("v1"), Some(SchemaVersion::new(1, 0)));
        assert_eq!(SchemaVersion::parse("v1.2"), Some(SchemaVersion::new(1, 2)));
        assert_eq!(SchemaVersion::parse("1.0"), Some(SchemaVersion::new(1, 0)));
        assert_eq!(SchemaVersion::parse("2"), Some(SchemaVersion::new(2, 0)));
        assert_eq!(SchemaVersion::parse("invalid"), None);
        assert_eq!(SchemaVersion::parse("v"), None);
    }

    #[test]
    fn version_display() {
        assert_eq!(SchemaVersion::new(1, 0).to_string(), "v1.0");
        assert_eq!(SchemaVersion::new(2, 3).to_string(), "v2.3");
        assert_eq!(SchemaVersion::new(1, 0).to_short_string(), "v1");
        assert_eq!(SchemaVersion::new(1, 1).to_short_string(), "v1.1");
    }

    #[test]
    fn version_compatibility() {
        let v1_0 = SchemaVersion::new(1, 0);
        let v1_1 = SchemaVersion::new(1, 1);
        let v2_0 = SchemaVersion::new(2, 0);

        // Same major, higher minor = compatible
        assert!(v1_1.is_compatible_with(&v1_0));

        // Same major, lower minor = not compatible
        assert!(!v1_0.is_compatible_with(&v1_1));

        // Same version = compatible
        assert!(v1_0.is_compatible_with(&v1_0));

        // Different major = not compatible
        assert!(!v2_0.is_compatible_with(&v1_0));
        assert!(!v1_0.is_compatible_with(&v2_0));
    }

    #[test]
    fn version_ordering() {
        let v1_0 = SchemaVersion::new(1, 0);
        let v1_1 = SchemaVersion::new(1, 1);
        let v2_0 = SchemaVersion::new(2, 0);

        assert!(v1_0 < v1_1);
        assert!(v1_1 < v2_0);
        assert!(v1_0 < v2_0);

        let mut versions = vec![v2_0, v1_0, v1_1];
        versions.sort();
        assert_eq!(versions, vec![v1_0, v1_1, v2_0]);
    }

    #[test]
    fn version_successor() {
        let v1_0 = SchemaVersion::new(1, 0);
        let v1_1 = SchemaVersion::new(1, 1);
        let v2_0 = SchemaVersion::new(2, 0);

        assert!(v1_1.is_successor_of(&v1_0));
        assert!(v2_0.is_successor_of(&v1_1));
        assert!(v2_0.is_successor_of(&v1_0));
        assert!(!v1_0.is_successor_of(&v1_1));
    }

    #[test]
    fn version_range_contains() {
        let range = VersionRange::between(SchemaVersion::new(1, 0), SchemaVersion::new(1, 5));

        assert!(range.contains(SchemaVersion::new(1, 0)));
        assert!(range.contains(SchemaVersion::new(1, 3)));
        assert!(range.contains(SchemaVersion::new(1, 5)));
        assert!(!range.contains(SchemaVersion::new(0, 9)));
        assert!(!range.contains(SchemaVersion::new(1, 6)));
        assert!(!range.contains(SchemaVersion::new(2, 0)));
    }

    #[test]
    fn version_range_from() {
        let range = VersionRange::from(SchemaVersion::new(1, 0));

        assert!(!range.contains(SchemaVersion::new(0, 9)));
        assert!(range.contains(SchemaVersion::new(1, 0)));
        assert!(range.contains(SchemaVersion::new(2, 5)));
        assert!(range.contains(SchemaVersion::new(100, 0)));
    }

    #[test]
    fn version_range_exact() {
        let range = VersionRange::exact(SchemaVersion::new(1, 2));

        assert!(!range.contains(SchemaVersion::new(1, 1)));
        assert!(range.contains(SchemaVersion::new(1, 2)));
        assert!(!range.contains(SchemaVersion::new(1, 3)));
    }

    #[test]
    fn version_range_overlaps() {
        let range1 = VersionRange::between(SchemaVersion::new(1, 0), SchemaVersion::new(1, 5));
        let range2 = VersionRange::between(SchemaVersion::new(1, 3), SchemaVersion::new(2, 0));
        let range3 = VersionRange::between(SchemaVersion::new(2, 0), SchemaVersion::new(3, 0));

        assert!(range1.overlaps(&range2)); // 1.3-1.5 overlap
        assert!(range2.overlaps(&range3)); // 2.0 overlap
        assert!(!range1.overlaps(&range3)); // No overlap
    }

    #[test]
    fn change_kind_breaking() {
        assert!(!ChangeKind::AddOptionalField.is_breaking());
        assert!(ChangeKind::AddRequiredField.is_breaking());
        assert!(ChangeKind::RemoveField.is_breaking());
        assert!(ChangeKind::ChangeFieldType.is_breaking());
        assert!(ChangeKind::RenameField.is_breaking());
        assert!(!ChangeKind::DeprecateField.is_breaking());
        assert!(!ChangeKind::ChangeDefault.is_breaking());
        assert!(!ChangeKind::MakeOptional.is_breaking());
        assert!(ChangeKind::MakeRequired.is_breaking());
    }
}
