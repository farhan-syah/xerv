//! Custom Resource Definitions for XERV Kubernetes operator.
//!
//! This module defines the CRDs that the operator manages:
//!
//! - [`XervCluster`]: A XERV cluster deployment
//! - [`XervPipeline`]: A pipeline deployed to a cluster

mod cluster;
mod pipeline;

pub use cluster::{
    ClusterCondition, ClusterPhase, DispatchSpec, NatsSpec, RedisSpec, ResourceRequirements,
    ResourceSpec, StorageSpec, Toleration, XervCluster, XervClusterSpec, XervClusterStatus,
};
pub use pipeline::{
    GitSource, PipelineCondition, PipelinePhase, PipelineSource, TriggerSpec, XervPipeline,
    XervPipelineSpec, XervPipelineStatus,
};
