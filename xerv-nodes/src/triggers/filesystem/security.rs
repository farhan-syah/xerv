//! Security configuration for filesystem path validation.

use std::path::{Path, PathBuf};
use xerv_core::error::{Result, XervError};

/// Security configuration for filesystem path validation.
#[derive(Debug, Clone)]
pub struct PathSecurityConfig {
    /// Paths that are explicitly allowed (whitelist mode).
    /// If non-empty, ONLY these paths (and their subdirectories) can be watched.
    pub allowed_paths: Vec<PathBuf>,
    /// Paths that are explicitly blocked (blacklist mode).
    /// Applied when allowed_paths is empty.
    pub blocked_paths: Vec<PathBuf>,
    /// Whether to allow watching paths outside user home directories.
    /// When false, restricts to paths under $HOME or allowed_paths.
    pub allow_system_paths: bool,
}

impl Default for PathSecurityConfig {
    fn default() -> Self {
        Self {
            allowed_paths: Vec::new(),
            blocked_paths: vec![
                // System configuration
                PathBuf::from("/etc"),
                PathBuf::from("/etc/passwd"),
                PathBuf::from("/etc/shadow"),
                PathBuf::from("/etc/sudoers"),
                // Root home
                PathBuf::from("/root"),
                // System logs (may contain sensitive data)
                PathBuf::from("/var/log"),
                // Kernel interfaces
                PathBuf::from("/proc"),
                PathBuf::from("/sys"),
                // Device files
                PathBuf::from("/dev"),
                // Boot files
                PathBuf::from("/boot"),
                // Package manager data
                PathBuf::from("/var/lib/dpkg"),
                PathBuf::from("/var/lib/rpm"),
                // SSH keys
                PathBuf::from("/home/*/.ssh"),
                // Private keys patterns
                PathBuf::from("/var/lib/private"),
            ],
            allow_system_paths: false,
        }
    }
}

impl PathSecurityConfig {
    /// Create a new security config with custom settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an allowed path (whitelist mode).
    pub fn allow_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_paths.push(path.into());
        self
    }

    /// Add a blocked path (blacklist mode).
    pub fn block_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.blocked_paths.push(path.into());
        self
    }

    /// Allow watching system paths outside home directories.
    pub fn allow_system_paths(mut self) -> Self {
        self.allow_system_paths = true;
        self
    }

    /// Validate a path against security rules.
    ///
    /// Returns Ok(()) if the path is allowed, or an error describing why it's blocked.
    pub fn validate_path(&self, path: &Path) -> Result<()> {
        // Canonicalize to resolve symlinks and prevent traversal attacks
        let canonical = path.canonicalize().map_err(|e| {
            Self::path_error(format!("Cannot resolve path '{}': {}", path.display(), e))
        })?;

        // Whitelist mode: if allowed_paths is non-empty, path must be under one of them
        if !self.allowed_paths.is_empty() {
            let is_allowed = self
                .allowed_paths
                .iter()
                .any(|allowed| canonical.starts_with(allowed) || canonical == *allowed);

            if !is_allowed {
                return Err(Self::path_error(format!(
                    "Path '{}' is not in the allowed paths list. Allowed: {:?}",
                    canonical.display(),
                    self.allowed_paths
                )));
            }
            return Ok(());
        }

        // Blacklist mode: check if path is in blocked list
        for blocked in &self.blocked_paths {
            let blocked_str = blocked.to_string_lossy();

            if blocked_str.contains('*') {
                // Use proper glob matching for patterns with wildcards
                if Self::glob_matches(&blocked_str, &canonical) {
                    return Err(Self::path_error(format!(
                        "Path '{}' matches blocked pattern '{}' (security restriction)",
                        canonical.display(),
                        blocked.display()
                    )));
                }
            } else if canonical.starts_with(blocked) || canonical == *blocked {
                return Err(Self::path_error(format!(
                    "Path '{}' is blocked for security reasons (matches '{}')",
                    canonical.display(),
                    blocked.display()
                )));
            }
        }

        // If system paths are not allowed, check if path is under a known safe location
        if !self.allow_system_paths {
            // Allow paths under home directories
            if let Some(home) = dirs::home_dir() {
                if canonical.starts_with(&home) {
                    return Ok(());
                }
            }

            // Allow paths under /tmp
            if canonical.starts_with("/tmp") || canonical.starts_with("/var/tmp") {
                return Ok(());
            }

            // Allow paths under common data directories
            let safe_prefixes = ["/data", "/srv", "/opt", "/usr/local/share", "/var/data"];

            for prefix in safe_prefixes {
                if canonical.starts_with(prefix) {
                    return Ok(());
                }
            }

            return Err(Self::path_error(format!(
                "Path '{}' is outside allowed directories. Use allowed_paths to whitelist \
                 specific paths, or set allow_system_paths: true in security config",
                canonical.display()
            )));
        }

        Ok(())
    }

    /// Create a standardized path validation error.
    ///
    /// This helper ensures consistent error formatting across all path validation.
    /// It is public within the crate for use by `FilesystemTrigger` validation.
    pub(crate) fn path_error(message: impl Into<String>) -> XervError {
        XervError::ConfigValue {
            field: "path".to_string(),
            cause: message.into(),
        }
    }

    /// Match a glob pattern against a path.
    ///
    /// Supports `*` wildcard which matches exactly one path component (not crossing `/`).
    ///
    /// # Examples
    ///
    /// - `/home/*/.ssh` matches `/home/alice/.ssh` and `/home/bob/.ssh`
    /// - `/home/*/.ssh` does NOT match `/home/alice/subdir/.ssh`
    /// - `/var/*/data` matches `/var/lib/data` but not `/var/lib/sub/data`
    fn glob_matches(pattern: &str, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Split pattern and path into components
        let pattern_parts: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        let path_parts: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();

        Self::glob_match_parts(&pattern_parts, &path_parts)
    }

    /// Recursively match pattern parts against path parts.
    fn glob_match_parts(pattern: &[&str], path: &[&str]) -> bool {
        match (pattern.first(), path.first()) {
            // Both exhausted: match
            (None, None) => true,

            // Pattern exhausted but path continues: check if path is under matched directory
            (None, Some(_)) => true, // Pattern `/home/*/.ssh` should match `/home/alice/.ssh/id_rsa`

            // Path exhausted but pattern continues: no match
            (Some(_), None) => false,

            // Both have elements
            (Some(&pat), Some(&p)) => {
                if pat == "*" {
                    // Wildcard matches any single component
                    Self::glob_match_parts(&pattern[1..], &path[1..])
                } else if pat == p {
                    // Exact match
                    Self::glob_match_parts(&pattern[1..], &path[1..])
                } else {
                    // No match
                    false
                }
            }
        }
    }
}

/// Global security configuration for filesystem triggers.
///
/// This singleton can be set once at application startup to enforce
/// organization-wide security policies for all filesystem triggers.
static GLOBAL_PATH_SECURITY: std::sync::OnceLock<PathSecurityConfig> = std::sync::OnceLock::new();

/// Set the global path security configuration.
///
/// This function configures security policies for all filesystem triggers in the
/// application. It should be called **once** at application startup, before any
/// triggers are created.
///
/// # Thread Safety
///
/// This function uses `OnceLock` internally, making it safe to call from multiple
/// threads. However, only the first call will succeed; subsequent calls return
/// the configuration that was rejected.
///
/// # Example
///
/// ```ignore
/// use xerv_nodes::triggers::filesystem::{PathSecurityConfig, set_global_path_security};
///
/// // Configure allowed paths at application startup
/// let config = PathSecurityConfig::new()
///     .allow_path("/data/workflows")
///     .allow_path("/data/uploads")
///     .allow_path("/tmp/xerv");
///
/// set_global_path_security(config)
///     .expect("Security config should only be set once");
/// ```
///
/// # Returns
///
/// - `Ok(())` if the configuration was successfully set
/// - `Err(config)` if a global configuration was already set (returns the rejected config)
///
/// # When to Use
///
/// Use this when you need to:
/// - Restrict filesystem triggers to specific directories (whitelist mode)
/// - Add custom paths to the blocklist
/// - Allow access to system paths that are blocked by default
///
/// If you don't call this function, the default [`PathSecurityConfig`] is used,
/// which blocks sensitive system directories and allows paths under home, `/tmp`,
/// and common data directories.
#[allow(dead_code)] // Public API for library consumers
pub fn set_global_path_security(
    config: PathSecurityConfig,
) -> std::result::Result<(), PathSecurityConfig> {
    GLOBAL_PATH_SECURITY.set(config)
}

/// Get the global path security configuration.
///
/// Returns the configuration set by [`set_global_path_security`], or the default
/// configuration if none was set.
///
/// # Default Configuration
///
/// The default configuration:
/// - Blocks: `/etc`, `/root`, `/proc`, `/sys`, `/dev`, `/boot`, `/var/log`
/// - Allows: Home directories, `/tmp`, `/data`, `/srv`, `/opt`
/// - Does not allow arbitrary system paths
pub fn global_path_security() -> &'static PathSecurityConfig {
    GLOBAL_PATH_SECURITY.get_or_init(PathSecurityConfig::default)
}
