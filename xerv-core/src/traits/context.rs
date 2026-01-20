//! Execution context provided to nodes.

use crate::arena::{ArenaReader, ArenaWriter};
use crate::error::Result;
use crate::testing::providers::RealSecrets;
use crate::testing::providers::{
    ClockProvider, EnvProvider, FsProvider, HttpProvider, RealClock, RealEnv, RealFs, RealHttp,
    RealRng, RealUuid, RngProvider, SecretsProvider, UuidProvider,
};
use crate::traits::trigger::TriggerType;
use crate::types::{ArenaOffset, NodeId, RelPtr, TraceId};
use crate::wal::Wal;
use std::sync::Arc;

/// Execution context provided to nodes during execution.
///
/// The context provides access to:
/// - Arena for reading/writing data
/// - Pipeline configuration
/// - Trace metadata
/// - Logging and metrics
/// - External providers (clock, HTTP, RNG, etc.)
///
/// # Providers
///
/// The context includes pluggable providers for external dependencies,
/// enabling deterministic testing by mocking these dependencies.
///
/// In production, real providers are used automatically. In tests,
/// use `TestContext` or the `with_providers()` constructor to inject mocks.
pub struct Context {
    /// The trace ID for this execution.
    trace_id: TraceId,
    /// The node ID being executed.
    node_id: NodeId,
    /// Arena reader for accessing upstream outputs.
    reader: ArenaReader,
    /// Arena writer for storing outputs.
    writer: ArenaWriter,
    /// WAL for durability.
    wal: Arc<Wal>,

    // Providers for external dependencies
    /// Clock provider for time operations.
    clock: Arc<dyn ClockProvider>,
    /// HTTP provider for network requests.
    http: Arc<dyn HttpProvider>,
    /// RNG provider for random number generation.
    rng: Arc<dyn RngProvider>,
    /// UUID provider for UUID generation.
    uuid: Arc<dyn UuidProvider>,
    /// Filesystem provider for file operations.
    fs: Arc<dyn FsProvider>,
    /// Environment provider for environment variables.
    env: Arc<dyn EnvProvider>,
    /// Secrets provider for secret management.
    secrets: Arc<dyn SecretsProvider>,
}

impl Context {
    /// Create a new execution context with default (real) providers.
    pub fn new(
        trace_id: TraceId,
        node_id: NodeId,
        reader: ArenaReader,
        writer: ArenaWriter,
        wal: Arc<Wal>,
    ) -> Self {
        Self {
            trace_id,
            node_id,
            reader,
            writer,
            wal,
            clock: Arc::new(RealClock::new()),
            http: Arc::new(RealHttp::new()),
            rng: Arc::new(RealRng::new()),
            uuid: Arc::new(RealUuid::new()),
            fs: Arc::new(RealFs::new()),
            env: Arc::new(RealEnv::new()),
            secrets: Arc::new(RealSecrets::default()),
        }
    }

    /// Create a new execution context with custom providers.
    ///
    /// This is primarily used for testing to inject mock providers.
    #[allow(clippy::too_many_arguments)]
    pub fn with_providers(
        trace_id: TraceId,
        node_id: NodeId,
        reader: ArenaReader,
        writer: ArenaWriter,
        wal: Arc<Wal>,
        clock: Arc<dyn ClockProvider>,
        http: Arc<dyn HttpProvider>,
        rng: Arc<dyn RngProvider>,
        uuid: Arc<dyn UuidProvider>,
        fs: Arc<dyn FsProvider>,
        env: Arc<dyn EnvProvider>,
        secrets: Arc<dyn SecretsProvider>,
    ) -> Self {
        Self {
            trace_id,
            node_id,
            reader,
            writer,
            wal,
            clock,
            http,
            rng,
            uuid,
            fs,
            env,
            secrets,
        }
    }

    /// Get the current trace ID.
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Get the current node ID.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Read bytes from the arena using a relative pointer.
    pub fn read_bytes(&self, ptr: RelPtr<()>) -> Result<Vec<u8>> {
        self.reader.read_bytes(ptr.offset(), ptr.size() as usize)
    }

    /// Write bytes to the arena and return a relative pointer.
    pub fn write_bytes(&self, bytes: &[u8]) -> Result<RelPtr<()>> {
        self.writer.write_bytes(bytes)
    }

    /// Read raw bytes from the arena at a specific offset.
    pub fn read_raw(&self, offset: ArenaOffset, size: usize) -> Result<Vec<u8>> {
        self.reader.read_bytes(offset, size)
    }

    /// Write raw bytes to the arena.
    pub fn write_raw(&self, bytes: &[u8]) -> Result<RelPtr<()>> {
        self.writer.write_bytes(bytes)
    }

    /// Log a message associated with this execution.
    pub fn log(&self, message: impl AsRef<str>) {
        tracing::info!(
            trace_id = %self.trace_id,
            node_id = %self.node_id,
            "{}",
            message.as_ref()
        );
    }

    /// Log a warning message.
    pub fn warn(&self, message: impl AsRef<str>) {
        tracing::warn!(
            trace_id = %self.trace_id,
            node_id = %self.node_id,
            "{}",
            message.as_ref()
        );
    }

    /// Log an error message.
    pub fn error(&self, message: impl AsRef<str>) {
        tracing::error!(
            trace_id = %self.trace_id,
            node_id = %self.node_id,
            "{}",
            message.as_ref()
        );
    }

    /// Get the current write position in the arena.
    pub fn write_position(&self) -> ArenaOffset {
        self.writer.write_position()
    }

    /// Get the WAL handle.
    pub fn wal(&self) -> &Wal {
        &self.wal
    }

    // Provider accessors

    /// Get the clock provider.
    pub fn clock(&self) -> &dyn ClockProvider {
        &*self.clock
    }

    /// Get the HTTP provider.
    pub fn http(&self) -> &dyn HttpProvider {
        &*self.http
    }

    /// Get the RNG provider.
    pub fn rng(&self) -> &dyn RngProvider {
        &*self.rng
    }

    /// Get the UUID provider.
    pub fn uuid_provider(&self) -> &dyn UuidProvider {
        &*self.uuid
    }

    /// Get the filesystem provider.
    pub fn fs(&self) -> &dyn FsProvider {
        &*self.fs
    }

    /// Get the environment provider.
    pub fn env_provider(&self) -> &dyn EnvProvider {
        &*self.env
    }

    /// Get the secrets provider.
    pub fn secrets(&self) -> &dyn SecretsProvider {
        &*self.secrets
    }

    // Convenience methods that use providers

    /// Get current time in nanoseconds (monotonic).
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Get current system time in milliseconds since UNIX epoch.
    pub fn system_time_millis(&self) -> u64 {
        self.clock.system_time_millis()
    }

    /// Generate a new UUID.
    pub fn new_uuid(&self) -> uuid::Uuid {
        self.uuid.new_v4()
    }

    /// Generate a random u64.
    pub fn random_u64(&self) -> u64 {
        self.rng.next_u64()
    }

    /// Generate a random f64 in [0, 1).
    pub fn random_f64(&self) -> f64 {
        self.rng.next_f64()
    }

    /// Get an environment variable.
    pub fn env_var(&self, key: &str) -> Option<String> {
        self.env.var(key)
    }

    /// Get a secret.
    pub fn secret(&self, key: &str) -> Option<String> {
        self.secrets.get(key)
    }
}

/// Context for pipeline lifecycle hooks.
#[derive(Clone)]
pub struct PipelineCtx {
    /// Pipeline name.
    pub name: String,
    /// Pipeline version.
    pub version: u32,
    /// Trigger controller.
    pub triggers: TriggerController,
}

impl PipelineCtx {
    /// Create a new pipeline context.
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        Self {
            name: name.into(),
            version,
            triggers: TriggerController::new(),
        }
    }

    /// Log a message for the pipeline.
    pub fn log(&self, message: impl AsRef<str>) {
        tracing::info!(
            pipeline = %self.name,
            version = %self.version,
            "{}",
            message.as_ref()
        );
    }
}

/// Controller for managing triggers within a pipeline.
#[derive(Clone)]
pub struct TriggerController {
    // Internal state for trigger management
    _private: (),
}

impl TriggerController {
    /// Create a new trigger controller.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Pause a specific trigger type.
    pub fn pause(&self, _trigger_type: TriggerType) {
        // Implementation will be in the executor
        tracing::info!("Pausing trigger");
    }

    /// Resume a specific trigger type.
    pub fn resume(&self, _trigger_type: TriggerType) {
        tracing::info!("Resuming trigger");
    }

    /// Pause all triggers.
    pub fn pause_all(&self) {
        tracing::info!("Pausing all triggers");
    }

    /// Resume all triggers.
    pub fn resume_all(&self) {
        tracing::info!("Resuming all triggers");
    }
}

impl Default for TriggerController {
    fn default() -> Self {
        Self::new()
    }
}
