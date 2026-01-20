//! Test context and builder for deterministic testing.
//!
//! Provides a configurable context for testing flows with mock providers.

use super::providers::{
    ClockProvider, EnvProvider, FsProvider, HttpProvider, MockClock, MockEnv, MockFs, MockHttp,
    MockRng, MockSecrets, MockUuid, RealClock, RealEnv, RealFs, RealHttp, RealRng, RealUuid,
    RngProvider, SecretsProvider, UuidProvider,
};
use super::recording::EventRecorder;
use crate::arena::{ArenaConfig, ArenaReader, ArenaWriter};
use crate::error::Result;
use crate::types::{ArenaOffset, NodeId, RelPtr, TraceId};
use crate::wal::{Wal, WalConfig};
use std::sync::Arc;

/// Test context with pluggable providers for deterministic testing.
///
/// This context provides access to all the same functionality as the regular
/// `Context`, plus mock providers for external dependencies.
///
/// # Example
///
/// ```ignore
/// use xerv_core::testing::{TestContextBuilder, MockClock, MockHttp};
///
/// let ctx = TestContextBuilder::new()
///     .with_fixed_time("2024-01-15T10:30:00Z")
///     .with_seed(42)
///     .with_sequential_uuids()
///     .with_recording()
///     .build()
///     .unwrap();
///
/// // Use providers
/// let time = ctx.clock.system_time_millis();
/// let uuid = ctx.uuid.new_v4();
/// ```
pub struct TestContext {
    // Core context fields
    trace_id: TraceId,
    node_id: NodeId,
    reader: ArenaReader,
    writer: ArenaWriter,
    wal: Arc<Wal>,

    // Providers
    /// Clock provider for time operations.
    pub clock: Arc<dyn ClockProvider>,
    /// HTTP provider for network requests.
    pub http: Arc<dyn HttpProvider>,
    /// RNG provider for random number generation.
    pub rng: Arc<dyn RngProvider>,
    /// UUID provider for UUID generation.
    pub uuid: Arc<dyn UuidProvider>,
    /// Filesystem provider for file operations.
    pub fs: Arc<dyn FsProvider>,
    /// Environment provider for environment variables.
    pub env: Arc<dyn EnvProvider>,
    /// Secrets provider for secret management.
    pub secrets: Arc<dyn SecretsProvider>,

    // Recording
    recorder: Option<Arc<EventRecorder>>,
}

impl TestContext {
    /// Get the trace ID.
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Get the node ID.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Read bytes from the arena.
    pub fn read_bytes(&self, ptr: RelPtr<()>) -> Result<Vec<u8>> {
        self.reader.read_bytes(ptr.offset(), ptr.size() as usize)
    }

    /// Write bytes to the arena.
    pub fn write_bytes(&self, bytes: &[u8]) -> Result<RelPtr<()>> {
        self.writer.write_bytes(bytes)
    }

    /// Read raw bytes from the arena.
    pub fn read_raw(&self, offset: ArenaOffset, size: usize) -> Result<Vec<u8>> {
        self.reader.read_bytes(offset, size)
    }

    /// Write raw bytes to the arena.
    pub fn write_raw(&self, bytes: &[u8]) -> Result<RelPtr<()>> {
        self.writer.write_bytes(bytes)
    }

    /// Log a message.
    pub fn log(&self, message: impl AsRef<str>) {
        tracing::info!(
            trace_id = %self.trace_id,
            node_id = %self.node_id,
            "{}",
            message.as_ref()
        );
    }

    /// Log a warning.
    pub fn warn(&self, message: impl AsRef<str>) {
        tracing::warn!(
            trace_id = %self.trace_id,
            node_id = %self.node_id,
            "{}",
            message.as_ref()
        );
    }

    /// Log an error.
    pub fn error(&self, message: impl AsRef<str>) {
        tracing::error!(
            trace_id = %self.trace_id,
            node_id = %self.node_id,
            "{}",
            message.as_ref()
        );
    }

    /// Get the current write position.
    pub fn write_position(&self) -> ArenaOffset {
        self.writer.write_position()
    }

    /// Get the WAL.
    pub fn wal(&self) -> &Wal {
        &self.wal
    }

    /// Get the event recorder (if recording is enabled).
    pub fn recorder(&self) -> Option<&Arc<EventRecorder>> {
        self.recorder.as_ref()
    }

    /// Record an event (if recording is enabled).
    pub fn record(&self, event: super::recording::RecordedEvent) {
        if let Some(recorder) = &self.recorder {
            recorder.record(event);
        }
    }

    // Provider-based operations with recording

    /// Get current time in nanoseconds.
    pub fn now(&self) -> u64 {
        let nanos = self.clock.now();
        self.record(super::recording::RecordedEvent::ClockNow { nanos });
        nanos
    }

    /// Get current system time in milliseconds.
    pub fn system_time_millis(&self) -> u64 {
        let millis = self.clock.system_time_millis();
        self.record(super::recording::RecordedEvent::SystemTime { millis });
        millis
    }

    /// Generate a new UUID.
    pub fn new_uuid(&self) -> uuid::Uuid {
        let uuid = self.uuid.new_v4();
        self.record(super::recording::RecordedEvent::UuidGenerated {
            uuid: uuid.to_string(),
        });
        uuid
    }

    /// Generate a random u64.
    pub fn random_u64(&self) -> u64 {
        let value = self.rng.next_u64();
        self.record(super::recording::RecordedEvent::RandomU64 { value });
        value
    }

    /// Generate a random f64 in [0, 1).
    pub fn random_f64(&self) -> f64 {
        let value = self.rng.next_f64();
        self.record(super::recording::RecordedEvent::RandomF64 { value });
        value
    }

    /// Get an environment variable.
    pub fn env_var(&self, key: &str) -> Option<String> {
        let value = self.env.var(key);
        self.record(super::recording::RecordedEvent::EnvRead {
            key: key.to_string(),
            value: value.clone(),
        });
        value
    }

    /// Get a secret.
    pub fn secret(&self, key: &str) -> Option<String> {
        let value = self.secrets.get(key);
        self.record(super::recording::RecordedEvent::SecretRead {
            key: key.to_string(),
            found: value.is_some(),
        });
        value
    }
}

/// Builder for creating test contexts with configurable providers.
///
/// # Example
///
/// ```ignore
/// use xerv_core::testing::TestContextBuilder;
///
/// let ctx = TestContextBuilder::new()
///     .with_fixed_time("2024-01-15T10:30:00Z")
///     .with_mock_http(
///         MockHttp::new()
///             .on_get("https://api.example.com/status")
///             .respond_json(200, serde_json::json!({"ok": true}))
///     )
///     .with_seed(42)
///     .with_sequential_uuids()
///     .with_env_vars(&[("DEBUG", "true")])
///     .with_secrets(&[("API_KEY", "test-key")])
///     .with_recording()
///     .build()
///     .unwrap();
/// ```
pub struct TestContextBuilder {
    trace_id: Option<TraceId>,
    node_id: Option<NodeId>,
    arena_config: ArenaConfig,
    wal_config: WalConfig,

    clock: Option<Arc<dyn ClockProvider>>,
    http: Option<Arc<dyn HttpProvider>>,
    rng: Option<Arc<dyn RngProvider>>,
    uuid: Option<Arc<dyn UuidProvider>>,
    fs: Option<Arc<dyn FsProvider>>,
    env: Option<Arc<dyn EnvProvider>>,
    secrets: Option<Arc<dyn SecretsProvider>>,

    recording: bool,
}

impl TestContextBuilder {
    /// Create a new test context builder with default settings.
    pub fn new() -> Self {
        Self {
            trace_id: None,
            node_id: None,
            arena_config: ArenaConfig::in_memory(),
            wal_config: WalConfig::in_memory(),

            clock: None,
            http: None,
            rng: None,
            uuid: None,
            fs: None,
            env: None,
            secrets: None,

            recording: false,
        }
    }

    /// Set the trace ID.
    pub fn with_trace_id(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the node ID.
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set the arena configuration.
    pub fn with_arena_config(mut self, config: ArenaConfig) -> Self {
        self.arena_config = config;
        self
    }

    /// Set the WAL configuration.
    pub fn with_wal_config(mut self, config: WalConfig) -> Self {
        self.wal_config = config;
        self
    }

    // Clock providers

    /// Use a fixed time.
    pub fn with_fixed_time(mut self, iso_time: &str) -> Self {
        self.clock = Some(Arc::new(MockClock::fixed(iso_time)));
        self
    }

    /// Use a mock clock starting at time zero.
    pub fn with_mock_clock(mut self) -> Self {
        self.clock = Some(Arc::new(MockClock::new()));
        self
    }

    /// Use a custom clock provider.
    pub fn with_clock(mut self, clock: Arc<dyn ClockProvider>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Use the real system clock.
    pub fn with_real_clock(mut self) -> Self {
        self.clock = Some(Arc::new(RealClock::new()));
        self
    }

    // HTTP providers

    /// Use a mock HTTP provider.
    pub fn with_mock_http(mut self, mock: MockHttp) -> Self {
        self.http = Some(Arc::new(mock));
        self
    }

    /// Use a custom HTTP provider.
    pub fn with_http(mut self, http: Arc<dyn HttpProvider>) -> Self {
        self.http = Some(http);
        self
    }

    /// Use the real HTTP provider.
    pub fn with_real_http(mut self) -> Self {
        self.http = Some(Arc::new(RealHttp::new()));
        self
    }

    // RNG providers

    /// Use a seeded RNG for deterministic behavior.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = Some(Arc::new(MockRng::seeded(seed)));
        self
    }

    /// Use a custom RNG provider.
    pub fn with_rng(mut self, rng: Arc<dyn RngProvider>) -> Self {
        self.rng = Some(rng);
        self
    }

    /// Use the real RNG.
    pub fn with_real_rng(mut self) -> Self {
        self.rng = Some(Arc::new(RealRng::new()));
        self
    }

    // UUID providers

    /// Use sequential UUIDs (00000001, 00000002, ...).
    pub fn with_sequential_uuids(mut self) -> Self {
        self.uuid = Some(Arc::new(MockUuid::sequential()));
        self
    }

    /// Use predetermined UUIDs.
    pub fn with_predetermined_uuids(mut self, uuids: &[&str]) -> Self {
        self.uuid = Some(Arc::new(MockUuid::from_strings(uuids)));
        self
    }

    /// Use a custom UUID provider.
    pub fn with_uuid(mut self, uuid: Arc<dyn UuidProvider>) -> Self {
        self.uuid = Some(uuid);
        self
    }

    /// Use the real UUID provider.
    pub fn with_real_uuid(mut self) -> Self {
        self.uuid = Some(Arc::new(RealUuid::new()));
        self
    }

    // Filesystem providers

    /// Use an in-memory filesystem.
    pub fn with_memory_fs(mut self) -> Self {
        self.fs = Some(Arc::new(MockFs::new()));
        self
    }

    /// Use a mock filesystem with predefined files.
    pub fn with_mock_fs(mut self, mock: MockFs) -> Self {
        self.fs = Some(Arc::new(mock));
        self
    }

    /// Use a custom filesystem provider.
    pub fn with_fs(mut self, fs: Arc<dyn FsProvider>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Use the real filesystem.
    pub fn with_real_fs(mut self) -> Self {
        self.fs = Some(Arc::new(RealFs::new()));
        self
    }

    // Environment providers

    /// Use mock environment variables.
    pub fn with_env_vars(mut self, vars: &[(&str, &str)]) -> Self {
        self.env = Some(Arc::new(MockEnv::from_pairs(vars)));
        self
    }

    /// Use a mock environment provider.
    pub fn with_mock_env(mut self, mock: MockEnv) -> Self {
        self.env = Some(Arc::new(mock));
        self
    }

    /// Use a custom environment provider.
    pub fn with_env(mut self, env: Arc<dyn EnvProvider>) -> Self {
        self.env = Some(env);
        self
    }

    /// Use the real environment.
    pub fn with_real_env(mut self) -> Self {
        self.env = Some(Arc::new(RealEnv::new()));
        self
    }

    // Secrets providers

    /// Use mock secrets.
    pub fn with_secrets(mut self, secrets: &[(&str, &str)]) -> Self {
        self.secrets = Some(Arc::new(MockSecrets::from_pairs(secrets)));
        self
    }

    /// Use a mock secrets provider.
    pub fn with_mock_secrets(mut self, mock: MockSecrets) -> Self {
        self.secrets = Some(Arc::new(mock));
        self
    }

    /// Use a custom secrets provider.
    pub fn with_secrets_provider(mut self, secrets: Arc<dyn SecretsProvider>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    // Recording

    /// Enable event recording.
    pub fn with_recording(mut self) -> Self {
        self.recording = true;
        self
    }

    /// Build the test context.
    pub fn build(self) -> Result<TestContext> {
        let trace_id = self.trace_id.unwrap_or_default();
        let node_id = self.node_id.unwrap_or_else(|| NodeId::new(0));

        // Create arena
        let arena = crate::arena::Arena::create(trace_id, &self.arena_config)?;
        let reader = arena.reader();
        let writer = arena.writer();

        // Create WAL
        let wal = Arc::new(Wal::open(self.wal_config.clone())?);

        // Default providers
        let clock = self.clock.unwrap_or_else(|| Arc::new(MockClock::new()));
        let http = self.http.unwrap_or_else(|| Arc::new(MockHttp::new()));
        let rng = self.rng.unwrap_or_else(|| Arc::new(MockRng::seeded(0)));
        let uuid = self
            .uuid
            .unwrap_or_else(|| Arc::new(MockUuid::sequential()));
        let fs = self.fs.unwrap_or_else(|| Arc::new(MockFs::new()));
        let env = self.env.unwrap_or_else(|| Arc::new(MockEnv::new()));
        let secrets = self.secrets.unwrap_or_else(|| Arc::new(MockSecrets::new()));

        let recorder = if self.recording {
            Some(Arc::new(EventRecorder::new()))
        } else {
            None
        };

        Ok(TestContext {
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

            recorder,
        })
    }
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_default_context() {
        let ctx = TestContextBuilder::new().build().unwrap();

        // Verify defaults are mocks
        assert!(ctx.clock.is_mock());
        assert!(ctx.http.is_mock());
        assert!(ctx.rng.is_mock());
        assert!(ctx.uuid.is_mock());
        assert!(ctx.fs.is_mock());
        assert!(ctx.env.is_mock());
        assert!(ctx.secrets.is_mock());
    }

    #[test]
    fn build_with_fixed_time() {
        let ctx = TestContextBuilder::new()
            .with_fixed_time("2024-01-15T10:30:00Z")
            .build()
            .unwrap();

        // 2024-01-15T10:30:00Z = 1705314600000 ms since epoch
        assert_eq!(ctx.system_time_millis(), 1705314600000);
    }

    #[test]
    fn build_with_seed() {
        let ctx1 = TestContextBuilder::new().with_seed(42).build().unwrap();
        let ctx2 = TestContextBuilder::new().with_seed(42).build().unwrap();

        let v1 = ctx1.random_u64();
        let v2 = ctx2.random_u64();

        assert_eq!(v1, v2);
    }

    #[test]
    fn build_with_sequential_uuids() {
        let ctx = TestContextBuilder::new()
            .with_sequential_uuids()
            .build()
            .unwrap();

        let id1 = ctx.new_uuid();
        let id2 = ctx.new_uuid();

        assert_eq!(id1.to_string(), "00000000-0000-0000-0000-000000000001");
        assert_eq!(id2.to_string(), "00000000-0000-0000-0000-000000000002");
    }

    #[test]
    fn build_with_env_vars() {
        let ctx = TestContextBuilder::new()
            .with_env_vars(&[("FOO", "bar"), ("BAZ", "qux")])
            .build()
            .unwrap();

        assert_eq!(ctx.env_var("FOO"), Some("bar".to_string()));
        assert_eq!(ctx.env_var("BAZ"), Some("qux".to_string()));
        assert_eq!(ctx.env_var("MISSING"), None);
    }

    #[test]
    fn build_with_secrets() {
        let ctx = TestContextBuilder::new()
            .with_secrets(&[("API_KEY", "secret-123")])
            .build()
            .unwrap();

        assert_eq!(ctx.secret("API_KEY"), Some("secret-123".to_string()));
        assert_eq!(ctx.secret("MISSING"), None);
    }

    #[test]
    fn build_with_recording() {
        let ctx = TestContextBuilder::new().with_recording().build().unwrap();

        // Generate some events
        ctx.now();
        ctx.new_uuid();
        ctx.random_u64();

        let recorder = ctx.recorder().unwrap();
        assert_eq!(recorder.len(), 3);
        assert!(recorder.assert_recorded("clock_now"));
        assert!(recorder.assert_recorded("uuid_generated"));
        assert!(recorder.assert_recorded("random_u64"));
    }

    #[test]
    fn arena_operations() {
        let ctx = TestContextBuilder::new().build().unwrap();

        let data = b"hello world";
        let ptr = ctx.write_bytes(data).unwrap();

        let read_data = ctx.read_bytes(ptr).unwrap();
        assert_eq!(read_data, data);
    }
}
