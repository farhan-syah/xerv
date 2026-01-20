//! Testing utilities for flow execution with mock providers.
//!
//! This module provides `FlowRunner` for executing flows with mocked dependencies,
//! enabling deterministic end-to-end testing of pipelines.
//!
//! # Example
//!
//! ```ignore
//! use xerv_executor::testing::FlowRunner;
//! use xerv_core::testing::{MockClock, MockHttp, MockHttpRule};
//!
//! // Build a flow runner with mocks
//! let mut runner = FlowRunner::new()
//!     .with_mock_http(MockHttp::new()
//!         .on_get("/api/data").respond_json(r#"{"value": 42}"#)
//!     )
//!     .with_fixed_time("2024-01-15T10:00:00Z")
//!     .add_node(NodeId::new(0), Box::new(my_node))
//!     .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
//!     .build()?;
//!
//! // Run the flow
//! let result = runner.run(b"initial input").await?;
//!
//! // Assert on outputs
//! assert_eq!(result.completed_nodes.len(), 2);
//! assert!(runner.http.assert_request_made("GET", "/api/data"));
//! ```

mod runner;

pub use runner::{FlowResult, FlowRunner, FlowRunnerBuilder};
