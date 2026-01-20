//! Deterministic replay testing framework.
//!
//! This module provides a `TestContext` and associated mocking infrastructure
//! that enables reproducible tests by mocking external dependencies such as
//! time, HTTP requests, random number generation, and filesystem operations.
//!
//! # Overview
//!
//! The framework extends the existing `Context` with pluggable providers,
//! allowing tests to inject mocks while production code uses real implementations.
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::testing::{TestContextBuilder, MockClock, MockHttp};
//! use serde_json::json;
//!
//! #[tokio::test]
//! async fn test_order_flow() {
//!     let ctx = TestContextBuilder::new()
//!         .with_fixed_time("2024-01-15T10:30:00Z")
//!         .with_mock_http(
//!             MockHttp::new()
//!                 .on_post("https://api.stripe.com/charges")
//!                 .respond_json(200, json!({"id": "ch_123", "status": "succeeded"}))
//!         )
//!         .with_seed(42)
//!         .with_sequential_uuids()
//!         .with_recording()
//!         .build()
//!         .unwrap();
//!
//!     // Execute flow with test context
//!     // Assert on outputs and recorded events
//! }
//! ```

pub mod chaos;
pub mod context;
pub mod providers;
pub mod recording;
pub mod snapshot;

// Re-export main types at module root
pub use chaos::{ChaosConfig, ChaosEngine, ChaosFault};
pub use context::{TestContext, TestContextBuilder};
pub use providers::{
    ClockProvider, EnvProvider, FsProvider, HttpProvider, HttpResponse, MockClock, MockEnv, MockFs,
    MockHttp, MockHttpRule, MockRng, MockSecrets, MockUuid, RealClock, RealEnv, RealFs, RealHttp,
    RealRng, RealUuid, RngProvider, SecretsProvider, UuidProvider,
};
pub use recording::{EventRecorder, RecordedEvent};
pub use snapshot::assert_snapshot;
