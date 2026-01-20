//! Filesystem trigger (file watcher).
//!
//! Fires events when files are created, modified, or deleted.

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;

/// State for the filesystem trigger.
struct FilesystemState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
}

/// Filesystem event trigger.
///
/// Watches a directory for file changes and fires events.
///
/// # Configuration
///
/// ```yaml
/// triggers:
///   - id: file_watcher
///     type: trigger::filesystem
///     params:
///       path: "/data/incoming"
///       recursive: true
///       events:
///         - create
///         - modify
/// ```
///
/// # Parameters
///
/// - `path` - Directory path to watch (required)
/// - `recursive` - Watch subdirectories (default: false)
/// - `events` - Event types to watch (default: all)
///   - `create` - File created
///   - `modify` - File modified
///   - `remove` - File deleted
///   - `rename` - File renamed
pub struct FilesystemTrigger {
    /// Trigger ID.
    id: String,
    /// Path to watch.
    path: PathBuf,
    /// Watch recursively.
    recursive: bool,
    /// Event types to watch.
    watch_create: bool,
    watch_modify: bool,
    watch_remove: bool,
    /// Internal state.
    state: Arc<FilesystemState>,
}

impl FilesystemTrigger {
    /// Create a new filesystem trigger.
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            recursive: false,
            watch_create: true,
            watch_modify: true,
            watch_remove: true,
            state: Arc::new(FilesystemState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
            }),
        }
    }

    /// Create from configuration.
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let path = config
            .get_string("path")
            .ok_or_else(|| XervError::ConfigValue {
                field: "path".to_string(),
                cause: "Filesystem trigger requires 'path' parameter".to_string(),
            })?;

        let recursive = config.get_bool("recursive").unwrap_or(false);

        // Parse event types
        let (watch_create, watch_modify, watch_remove) =
            if let Some(events) = config.params.get("events") {
                if let Some(events_arr) = events.as_sequence() {
                    let mut create = false;
                    let mut modify = false;
                    let mut remove = false;

                    for event in events_arr {
                        if let Some(event_str) = event.as_str() {
                            match event_str {
                                "create" => create = true,
                                "modify" => modify = true,
                                "remove" | "delete" => remove = true,
                                "rename" => {
                                    create = true;
                                    remove = true;
                                }
                                _ => {}
                            }
                        }
                    }

                    (create, modify, remove)
                } else {
                    (true, true, true)
                }
            } else {
                (true, true, true)
            };

        Ok(Self {
            id: config.id.clone(),
            path: PathBuf::from(path),
            recursive,
            watch_create,
            watch_modify,
            watch_remove,
            state: Arc::new(FilesystemState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
            }),
        })
    }

    /// Enable recursive watching.
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Watch only for create events.
    pub fn watch_create_only(mut self) -> Self {
        self.watch_create = true;
        self.watch_modify = false;
        self.watch_remove = false;
        self
    }
}

impl Trigger for FilesystemTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Filesystem
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn start<'a>(
        &'a self,
        callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let path = self.path.clone();
        let recursive = self.recursive;
        let trigger_id = self.id.clone();
        let watch_create = self.watch_create;
        let watch_modify = self.watch_modify;
        let watch_remove = self.watch_remove;

        Box::pin(async move {
            if state.running.load(Ordering::SeqCst) {
                return Err(XervError::ConfigValue {
                    field: "trigger".to_string(),
                    cause: "Trigger is already running".to_string(),
                });
            }

            // Check if path exists
            if !path.exists() {
                return Err(XervError::ConfigValue {
                    field: "path".to_string(),
                    cause: format!("Path does not exist: {}", path.display()),
                });
            }

            tracing::info!(
                trigger_id = %trigger_id,
                path = %path.display(),
                recursive = recursive,
                "Filesystem trigger started"
            );

            state.running.store(true, Ordering::SeqCst);

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write() = Some(shutdown_tx);

            let callback = Arc::new(callback);

            // Create channel for watcher events
            let (tx, mut rx) = mpsc::channel(100);

            // Create watcher
            let mut watcher = RecommendedWatcher::new(
                move |res: std::result::Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let _ = tx.blocking_send(event);
                    }
                },
                Config::default(),
            )
            .map_err(|e| XervError::Io {
                path: path.clone(),
                cause: format!("Failed to create file watcher: {}", e),
            })?;

            // Start watching
            let mode = if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };

            watcher.watch(&path, mode).map_err(|e| XervError::Io {
                path: path.clone(),
                cause: format!("Failed to watch path: {}", e),
            })?;

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!(trigger_id = %trigger_id, "Filesystem trigger shutting down");
                        break;
                    }
                    Some(event) = rx.recv() => {
                        if state.paused.load(Ordering::SeqCst) {
                            tracing::debug!(trigger_id = %trigger_id, "Trigger paused, ignoring event");
                            continue;
                        }

                        // Check event type
                        let should_trigger = match &event.kind {
                            notify::EventKind::Create(_) => watch_create,
                            notify::EventKind::Modify(_) => watch_modify,
                            notify::EventKind::Remove(_) => watch_remove,
                            _ => false,
                        };

                        if !should_trigger {
                            continue;
                        }

                        let paths: Vec<String> = event.paths.iter()
                            .map(|p| p.display().to_string())
                            .collect();

                        let event_kind = format!("{:?}", event.kind);

                        // Create trigger event
                        let trigger_event = TriggerEvent::new(&trigger_id, RelPtr::null())
                            .with_metadata(format!(
                                "event={},paths={}",
                                event_kind,
                                paths.join(",")
                            ));

                        tracing::debug!(
                            trigger_id = %trigger_id,
                            trace_id = %trigger_event.trace_id,
                            event_kind = %event_kind,
                            paths = ?paths,
                            "Filesystem event detected"
                        );

                        callback(trigger_event);
                    }
                }
            }

            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn stop<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            if let Some(tx) = state.shutdown_tx.write().take() {
                let _ = tx.send(());
                tracing::info!(trigger_id = %trigger_id, "Filesystem trigger stopped");
            }
            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(true, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Filesystem trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Filesystem trigger resumed");
            Ok(())
        })
    }

    fn is_running(&self) -> bool {
        self.state.running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_trigger_creation() {
        let trigger = FilesystemTrigger::new("test_fs", "/tmp");
        assert_eq!(trigger.id(), "test_fs");
        assert_eq!(trigger.trigger_type(), TriggerType::Filesystem);
        assert!(!trigger.is_running());
        assert!(!trigger.recursive);
    }

    #[test]
    fn filesystem_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/data/incoming".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("recursive".to_string()),
            serde_yaml::Value::Bool(true),
        );

        let config = TriggerConfig::new("fs_test", TriggerType::Filesystem)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = FilesystemTrigger::from_config(&config).unwrap();
        assert_eq!(trigger.id(), "fs_test");
        assert_eq!(trigger.path.to_str().unwrap(), "/data/incoming");
        assert!(trigger.recursive);
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

        assert!(trigger.recursive);
        assert!(trigger.watch_create);
        assert!(!trigger.watch_modify);
        assert!(!trigger.watch_remove);
    }
}
