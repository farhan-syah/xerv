//! Flow control nodes.
//!
//! These nodes control the execution flow of a pipeline:
//! - [`MergeNode`] - Barrier that waits for all inputs before continuing
//! - [`SwitchNode`] - Routes data based on a condition expression
//! - [`LoopNode`] - Controlled iteration with configurable exit conditions
//! - [`WaitNode`] - Human-in-the-loop approval patterns

mod loop_node;
mod merge;
mod switch;
mod wait;

pub use loop_node::{LoopCondition, LoopNode};
pub use merge::{MergeNode, MergeStrategy};
pub use switch::{SwitchCondition, SwitchNode};
pub use wait::{ResumeMethod, TimeoutAction, WaitNode};
