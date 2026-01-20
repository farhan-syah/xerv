//! Selector linker for resolving `${node.field}` expressions.
//!
//! The linker compiles selector expressions at flow load time into
//! direct memory access instructions.

mod parser;
mod resolver;

pub use parser::{Selector, SelectorParser};
pub use resolver::{CompiledSelector, Linker, NodeSchema, ResolvedField};
