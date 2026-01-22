//! Standard library nodes and triggers for XERV.
//!
//! This crate provides the built-in nodes and triggers that form XERV's standard library:
//!
//! ## Flow Control (`flow::*`)
//! - [`flow::MergeNode`] - N→1 barrier that waits for all inputs
//! - [`flow::SwitchNode`] - Conditional routing based on expression
//! - [`flow::LoopNode`] - Controlled iteration with exit condition
//! - [`flow::WaitNode`] - Human-in-the-loop approval patterns
//!
//! ## Data Manipulation (`data::*`)
//! - [`data::SplitNode`] - Fan-out iterator for collections
//! - [`data::MapNode`] - Field renaming and transformation
//! - [`data::ConcatNode`] - String concatenation
//! - [`data::AggregateNode`] - Numeric aggregation (sum, avg, min, max)
//! - [`data::JsonDynamicNode`] - Schemaless JSON handling
//!
//! ## Triggers (`triggers::*`)
//! - [`triggers::WebhookTrigger`] - HTTP webhook endpoint
//! - [`triggers::CronTrigger`] - Scheduled execution via cron expressions
//! - [`triggers::FilesystemTrigger`] - File system event watcher
//! - [`triggers::QueueTrigger`] - In-memory message queue
//! - [`triggers::MemoryTrigger`] - Direct memory injection (benchmarks)
//! - [`triggers::ManualTrigger`] - Manual invocation (testing)

pub mod data;
pub mod flow;
pub mod registry;
pub mod triggers;

// Flow control exports
pub use flow::{
    LoopCondition, LoopNode, MergeNode, MergeStrategy, ResumeMethod, SwitchCondition, SwitchNode,
    TimeoutAction, WaitNode,
};

// Data manipulation exports
pub use data::{
    AggregateNode, AggregateOperation, ConcatNode, FieldMapping, JsonDynamicNode, MapNode,
    SplitMode, SplitNode,
};

// Trigger exports
pub use triggers::{
    CronTrigger, CronTriggerFactory, FilesystemTrigger, FilesystemTriggerFactory, ManualEvent,
    ManualFireHandle, ManualTrigger, ManualTriggerFactory, MemoryInjector, MemoryTrigger,
    MemoryTriggerFactory, QueueHandle, QueueMessage, QueueTrigger, QueueTriggerFactory,
    StandardTriggerFactory, WebhookTrigger, WebhookTriggerFactory,
};

/// Prelude for commonly used types.
pub mod prelude {
    // Flow control
    pub use crate::flow::{
        LoopCondition, LoopNode, MergeNode, MergeStrategy, ResumeMethod, SwitchCondition,
        SwitchNode, TimeoutAction, WaitNode,
    };

    // Data manipulation
    pub use crate::data::{
        AggregateNode, AggregateOperation, ConcatNode, FieldMapping, JsonDynamicNode, MapNode,
        SplitMode, SplitNode,
    };

    // Triggers
    pub use crate::triggers::{
        CronTrigger, FilesystemTrigger, ManualEvent, ManualFireHandle, ManualTrigger,
        MemoryInjector, MemoryTrigger, QueueHandle, QueueMessage, QueueTrigger,
        StandardTriggerFactory, WebhookTrigger,
    };
}
