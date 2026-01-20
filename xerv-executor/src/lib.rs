//! XERV Executor - Trace execution engine.
//!
//! This crate provides the execution infrastructure for XERV:
//! - Topological scheduler with DAG support
//! - Selector linker for `${node.field}` resolution
//! - Pipeline lifecycle controller
//! - Trace state management
//! - Flow loader for YAML parsing and compilation
//! - Crash recovery and trace replay
//! - REST API for pipeline management
//! - Suspension system for human-in-the-loop workflows
//! - WASM node support (with `wasm` feature)

#![warn(missing_docs)]

pub mod api;
pub mod linker;
pub mod listener;
pub mod loader;
pub mod pipeline;
pub mod recovery;
pub mod scheduler;
pub mod suspension;
pub mod testing;
pub mod trace;

#[cfg(feature = "wasm")]
pub mod wasm;

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::api::{ApiError, ApiServer, AppState, ServerConfig, TraceHistory, TraceRecord};
    pub use crate::linker::{CompiledSelector, Linker, ResolvedField, Selector, SelectorParser};
    pub use crate::listener::{Listener, ListenerId, ListenerPool};
    pub use crate::loader::{FlowBuilder, FlowLoader, LoadedFlow, LoaderConfig, LoaderError};
    pub use crate::pipeline::{Pipeline, PipelineController, PipelineMetrics};
    pub use crate::recovery::{CrashReplayer, RecoveryAction, RecoveryReport};
    pub use crate::scheduler::{
        Edge, ExecutionSignal, Executor, ExecutorConfig, FlowGraph, GraphNode,
    };
    pub use crate::suspension::{
        MemorySuspensionStore, ResumeDecision, SuspendedTraceState, SuspensionRequest,
        SuspensionStore, TimeoutProcessor,
    };
    pub use crate::testing::{FlowResult, FlowRunner, FlowRunnerBuilder};
    pub use crate::trace::TraceState;

    #[cfg(feature = "wasm")]
    pub use crate::wasm::{
        WasmNode, WasmNodeConfig, WasmNodeFactory, WasmRuntime, WasmRuntimeConfig,
    };
}
