//! Filesystem trigger (file watcher).
//!
//! Fires events when files are created, modified, or deleted.
//!
//! # Security
//!
//! The filesystem trigger validates paths to prevent watching sensitive system
//! directories. By default, the following paths are blocked:
//!
//! - `/etc` - System configuration
//! - `/root` - Root home directory
//! - `/var/log` - System logs (may contain sensitive data)
//! - `/proc`, `/sys` - Kernel interfaces
//! - `/dev` - Device files
//! - `/boot` - Boot files
//!
//! Custom allowed paths can be configured via `PathSecurityConfig`.

mod security;
mod trigger;

pub use security::{PathSecurityConfig, global_path_security, set_global_path_security};
pub use trigger::FilesystemTrigger;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use xerv_core::traits::{Trigger, TriggerConfig, TriggerType};

    #[test]
    fn filesystem_trigger_creation() {
        let trigger = FilesystemTrigger::new("test_fs", "/tmp");
        assert_eq!(trigger.id(), "test_fs");
        assert_eq!(trigger.trigger_type(), TriggerType::Filesystem);
        assert!(!trigger.is_running());
    }

    #[test]
    fn filesystem_trigger_from_config_with_allowed_path() {
        // Create a security config that allows /tmp
        let security = PathSecurityConfig::new().allow_system_paths();

        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/tmp".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("recursive".to_string()),
            serde_yaml::Value::Bool(true),
        );

        let config = TriggerConfig::new("fs_test", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = FilesystemTrigger::from_config_with_security(&config, &security).unwrap();
        assert_eq!(trigger.id(), "fs_test");
    }

    #[test]
    fn filesystem_trigger_missing_path() {
        let config = TriggerConfig::new("fs_test", TriggerType::Filesystem);
        let result = FilesystemTrigger::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn filesystem_trigger_builder() {
        let trigger = FilesystemTrigger::new("builder_test", "/tmp")
            .recursive()
            .watch_create_only();

        assert!(trigger.is_running() == false);
    }

    // ========== Path Security Tests ==========

    #[test]
    fn path_security_blocks_etc() {
        let security = PathSecurityConfig::default();

        // /etc should be blocked
        let result = security.validate_path(Path::new("/etc"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn path_security_blocks_etc_passwd() {
        let security = PathSecurityConfig::default();

        // /etc/passwd should be blocked (under /etc)
        let result = security.validate_path(Path::new("/etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn path_security_blocks_root() {
        let security = PathSecurityConfig::default();

        let result = security.validate_path(Path::new("/root"));
        assert!(result.is_err());
    }

    #[test]
    fn path_security_blocks_proc() {
        let security = PathSecurityConfig::default();

        let result = security.validate_path(Path::new("/proc"));
        assert!(result.is_err());
    }

    #[test]
    fn path_security_allows_tmp() {
        let security = PathSecurityConfig::default();

        // /tmp should be allowed
        let result = security.validate_path(Path::new("/tmp"));
        assert!(result.is_ok());
    }

    #[test]
    fn path_security_whitelist_mode() {
        // Only allow /data/allowed
        let security = PathSecurityConfig::new().allow_path("/data/allowed");

        // Allowed path should work (if it exists, which it won't in tests)
        // So we test the logic indirectly
        assert!(!security.allowed_paths.is_empty());
        assert_eq!(
            security.allowed_paths[0],
            std::path::PathBuf::from("/data/allowed")
        );
    }

    #[test]
    fn path_security_custom_blocked() {
        let security = PathSecurityConfig::new()
            .block_path("/custom/blocked")
            .allow_system_paths(); // Allow system paths except our custom block

        // Our custom path should be in blocked list
        assert!(
            security
                .blocked_paths
                .iter()
                .any(|p| p == Path::new("/custom/blocked"))
        );
    }

    #[test]
    fn validate_path_syntax_rejects_traversal() {
        // Test via from_config which calls validate_path_syntax
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/data/../etc/passwd".to_string()),
        );

        let config = TriggerConfig::new("traversal", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let result = FilesystemTrigger::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }

    #[test]
    fn validate_path_syntax_rejects_null_bytes() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/data/file\0.txt".to_string()),
        );

        let config = TriggerConfig::new("null", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let result = FilesystemTrigger::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("null"));
    }

    #[test]
    fn validate_path_syntax_rejects_sensitive_prefix() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/etc/config.yaml".to_string()),
        );

        let config = TriggerConfig::new("sensitive", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let result = FilesystemTrigger::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sensitive"));
    }

    #[test]
    fn from_config_rejects_sensitive_path_in_yaml() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/etc/passwd".to_string()),
        );

        let config = TriggerConfig::new("malicious", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let result = FilesystemTrigger::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn from_config_rejects_path_traversal_in_yaml() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/data/../etc/shadow".to_string()),
        );

        let config = TriggerConfig::new("traversal", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let result = FilesystemTrigger::from_config(&config);
        assert!(result.is_err());
    }
}
