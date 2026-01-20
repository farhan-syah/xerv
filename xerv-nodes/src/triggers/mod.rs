//! Trigger implementations for XERV.
//!
//! Triggers are entry points that inject events into a flow.
//! This module provides the standard library triggers:
//!
//! - [`WebhookTrigger`] - HTTP webhook endpoint
//! - [`CronTrigger`] - Scheduled execution via cron expressions
//! - [`FilesystemTrigger`] - File system event watcher
//! - [`QueueTrigger`] - In-memory message queue
//! - [`MemoryTrigger`] - Direct memory injection (for benchmarks)
//! - [`ManualTrigger`] - Manual invocation (for testing)
//! - [`KafkaTrigger`] - Kafka consumer (requires `kafka` feature)

mod cron;
mod filesystem;
mod kafka;
mod manual;
mod memory;
mod queue;
mod webhook;

pub use self::cron::CronTrigger;
pub use filesystem::{
    FilesystemTrigger, PathSecurityConfig, global_path_security, set_global_path_security,
};
pub use kafka::KafkaTrigger;
pub use manual::{ManualEvent, ManualFireHandle, ManualTrigger};
pub use memory::{MemoryInjector, MemoryTrigger};
pub use queue::{QueueHandle, QueueMessage, QueueTrigger};
pub use webhook::WebhookTrigger;

use xerv_core::error::Result;
use xerv_core::traits::{Trigger, TriggerConfig, TriggerFactory, TriggerType};

/// Standard trigger factory that creates all built-in triggers.
pub struct StandardTriggerFactory;

impl StandardTriggerFactory {
    /// Create a new standard trigger factory.
    pub fn new() -> Self {
        Self
    }

    /// Create a trigger from configuration.
    pub fn create_trigger(config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        match config.trigger_type {
            TriggerType::Webhook => Ok(Box::new(WebhookTrigger::from_config(config)?)),
            TriggerType::Cron => Ok(Box::new(CronTrigger::from_config(config)?)),
            TriggerType::Filesystem => Ok(Box::new(FilesystemTrigger::from_config(config)?)),
            TriggerType::Queue => Ok(Box::new(QueueTrigger::from_config(config)?)),
            TriggerType::Memory => Ok(Box::new(MemoryTrigger::from_config(config)?)),
            TriggerType::Manual => Ok(Box::new(ManualTrigger::from_config(config)?)),
            TriggerType::Kafka => Ok(Box::new(KafkaTrigger::from_config(config)?)),
        }
    }
}

impl Default for StandardTriggerFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// Factory for webhook triggers.
pub struct WebhookTriggerFactory;

impl TriggerFactory for WebhookTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Webhook
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(WebhookTrigger::from_config(config)?))
    }
}

/// Factory for cron triggers.
pub struct CronTriggerFactory;

impl TriggerFactory for CronTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Cron
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(CronTrigger::from_config(config)?))
    }
}

/// Factory for filesystem triggers.
pub struct FilesystemTriggerFactory;

impl TriggerFactory for FilesystemTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Filesystem
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(FilesystemTrigger::from_config(config)?))
    }
}

/// Factory for queue triggers.
pub struct QueueTriggerFactory;

impl TriggerFactory for QueueTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Queue
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(QueueTrigger::from_config(config)?))
    }
}

/// Factory for memory triggers.
pub struct MemoryTriggerFactory;

impl TriggerFactory for MemoryTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Memory
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(MemoryTrigger::from_config(config)?))
    }
}

/// Factory for manual triggers.
pub struct ManualTriggerFactory;

impl TriggerFactory for ManualTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Manual
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(ManualTrigger::from_config(config)?))
    }
}

/// Factory for Kafka triggers.
pub struct KafkaTriggerFactory;

impl TriggerFactory for KafkaTriggerFactory {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Kafka
    }

    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>> {
        Ok(Box::new(KafkaTrigger::from_config(config)?))
    }
}
