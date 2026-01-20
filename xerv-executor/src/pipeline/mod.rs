//! Pipeline lifecycle controller.
//!
//! Manages pipeline states, concurrency, and deployment (blue/green).

mod controller;

pub use controller::{Pipeline, PipelineBuilder, PipelineController, PipelineMetrics};
