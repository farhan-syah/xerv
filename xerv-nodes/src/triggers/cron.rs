//! Cron trigger (scheduled execution).
//!
//! Fires events based on cron expressions.

use chrono::{DateTime, Utc};
use cron::Schedule;
use parking_lot::RwLock;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;

/// State for the cron trigger.
struct CronState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
}

/// Cron schedule trigger.
///
/// Fires events based on a cron expression.
///
/// # Configuration
///
/// ```yaml
/// triggers:
///   - id: daily_cleanup
///     type: trigger::cron
///     params:
///       schedule: "0 0 2 * * *"  # Every day at 2 AM
///       timezone: "UTC"
/// ```
///
/// # Parameters
///
/// - `schedule` - Cron expression (required)
/// - `timezone` - Timezone for scheduling (default: "UTC")
///
/// # Cron Expression Format
///
/// Standard 6-field cron format:
/// ```text
/// sec  min  hour  day  month  weekday
/// *    *    *     *    *      *
/// ```
///
/// Examples:
/// - `0 0 * * * *` - Every hour
/// - `0 30 9 * * Mon-Fri` - Weekdays at 9:30 AM
/// - `0 0 0 1 * *` - First day of each month at midnight
pub struct CronTrigger {
    /// Trigger ID.
    id: String,
    /// Cron schedule.
    schedule: Schedule,
    /// Original schedule expression (for display).
    schedule_expr: String,
    /// Internal state.
    state: Arc<CronState>,
}

impl CronTrigger {
    /// Create a new cron trigger.
    pub fn new(id: impl Into<String>, schedule_expr: impl Into<String>) -> Result<Self> {
        let schedule_str = schedule_expr.into();
        let schedule = Schedule::from_str(&schedule_str).map_err(|e| XervError::ConfigValue {
            field: "schedule".to_string(),
            cause: format!("Invalid cron expression '{}': {}", schedule_str, e),
        })?;

        Ok(Self {
            id: id.into(),
            schedule,
            schedule_expr: schedule_str,
            state: Arc::new(CronState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
            }),
        })
    }

    /// Create from configuration.
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let schedule_expr =
            config
                .get_string("schedule")
                .ok_or_else(|| XervError::ConfigValue {
                    field: "schedule".to_string(),
                    cause: "Cron trigger requires 'schedule' parameter".to_string(),
                })?;

        Self::new(&config.id, schedule_expr)
    }

    /// Get the next scheduled time.
    pub fn next_fire_time(&self) -> Option<DateTime<Utc>> {
        self.schedule.upcoming(Utc).next()
    }
}

impl Trigger for CronTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Cron
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn start<'a>(
        &'a self,
        callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let schedule = self.schedule.clone();
        let trigger_id = self.id.clone();
        let schedule_expr = self.schedule_expr.clone();

        Box::pin(async move {
            if state.running.load(Ordering::SeqCst) {
                return Err(XervError::ConfigValue {
                    field: "trigger".to_string(),
                    cause: "Trigger is already running".to_string(),
                });
            }

            tracing::info!(
                trigger_id = %trigger_id,
                schedule = %schedule_expr,
                "Cron trigger started"
            );

            state.running.store(true, Ordering::SeqCst);

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write() = Some(shutdown_tx);

            let callback = Arc::new(callback);

            loop {
                // Calculate next fire time
                let now = Utc::now();
                let next = match schedule.upcoming(Utc).next() {
                    Some(next) => next,
                    None => {
                        tracing::warn!(trigger_id = %trigger_id, "No more scheduled times");
                        break;
                    }
                };

                let duration = (next - now).to_std().unwrap_or(std::time::Duration::ZERO);

                tracing::debug!(
                    trigger_id = %trigger_id,
                    next_fire = %next,
                    wait_secs = duration.as_secs(),
                    "Waiting for next cron fire"
                );

                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!(trigger_id = %trigger_id, "Cron trigger shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(duration) => {
                        if state.paused.load(Ordering::SeqCst) {
                            tracing::debug!(trigger_id = %trigger_id, "Trigger paused, skipping fire");
                            continue;
                        }

                        // Create event
                        let event = TriggerEvent::new(&trigger_id, RelPtr::null())
                            .with_metadata(format!("scheduled_time={}", next));

                        tracing::info!(
                            trigger_id = %trigger_id,
                            trace_id = %event.trace_id,
                            scheduled_time = %next,
                            "Cron trigger fired"
                        );

                        callback(event);
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
                tracing::info!(trigger_id = %trigger_id, "Cron trigger stopped");
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
            tracing::info!(trigger_id = %trigger_id, "Cron trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Cron trigger resumed");
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
    fn cron_trigger_creation() {
        let trigger = CronTrigger::new("test_cron", "0 0 * * * *").unwrap();
        assert_eq!(trigger.id(), "test_cron");
        assert_eq!(trigger.trigger_type(), TriggerType::Cron);
        assert!(!trigger.is_running());
    }

    #[test]
    fn cron_trigger_invalid_expression() {
        let result = CronTrigger::new("test_cron", "invalid");
        assert!(result.is_err());
    }

    #[test]
    fn cron_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("schedule".to_string()),
            serde_yaml::Value::String("0 30 9 * * Mon-Fri".to_string()),
        );

        let config = TriggerConfig::new("cron_test", TriggerType::Cron)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = CronTrigger::from_config(&config).unwrap();
        assert_eq!(trigger.id(), "cron_test");
        assert_eq!(trigger.schedule_expr, "0 30 9 * * Mon-Fri");
    }

    #[test]
    fn cron_trigger_next_fire_time() {
        let trigger = CronTrigger::new("test", "0 * * * * *").unwrap();
        let next = trigger.next_fire_time();
        assert!(next.is_some());
    }

    #[test]
    fn cron_trigger_missing_schedule() {
        let config = TriggerConfig::new("cron_test", TriggerType::Cron);
        let result = CronTrigger::from_config(&config);
        assert!(result.is_err());
    }
}
