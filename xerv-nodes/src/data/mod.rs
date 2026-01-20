//! Data manipulation nodes.
//!
//! These nodes transform and process data within a pipeline:
//! - [`SplitNode`] - Fan-out iterator that processes each item in a collection
//! - [`MapNode`] - Transform/rename fields in the data
//! - [`ConcatNode`] - Combine multiple strings into one
//! - [`AggregateNode`] - Aggregate numeric values (sum, avg, min, max, count)
//! - [`JsonDynamicNode`] - Handle schemaless/dynamic JSON payloads

mod aggregate;
mod concat;
mod json_dynamic;
mod map;
mod split;

pub use aggregate::{AggregateNode, AggregateOperation};
pub use concat::ConcatNode;
pub use json_dynamic::JsonDynamicNode;
pub use map::{FieldMapping, MapNode};
pub use split::{SplitMode, SplitNode};
