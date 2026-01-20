//! XERV Kubernetes Operator
//!
//! This crate provides a Kubernetes operator for deploying and managing
//! XERV workflow orchestration clusters.
//!
//! # Custom Resource Definitions
//!
//! - **XervCluster**: Manages a XERV cluster (StatefulSet for Raft, Deployment for Redis/NATS)
//! - **XervPipeline**: Deploys a pipeline to a cluster
//!
//! # Example
//!
//! ```yaml
//! apiVersion: xerv.io/v1
//! kind: XervCluster
//! metadata:
//!   name: production
//! spec:
//!   replicas: 3
//!   dispatch:
//!     backend: raft
//!   storage:
//!     class: fast-ssd
//!     size: 10Gi
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod controller;
pub mod crd;
pub mod error;
pub mod resources;

pub use crd::{XervCluster, XervClusterSpec, XervPipeline, XervPipelineSpec};
pub use error::{OperatorError, OperatorResult};
