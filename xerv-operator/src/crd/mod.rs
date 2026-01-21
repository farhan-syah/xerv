//! Custom Resource Definitions for XERV Kubernetes operator.
//!
//! This module defines the CRDs that the operator manages:
//!
//! - [`XervCluster`]: A XERV cluster deployment
//! - [`XervPipeline`]: A pipeline deployed to a cluster
//! - [`XervFederation`]: A federation of XERV clusters across regions
//! - [`FederatedPipeline`]: A pipeline deployed across multiple federated clusters

mod cluster;
mod federated_pipeline;
mod federation;
mod pipeline;

pub use cluster::{
    ClusterCondition, ClusterPhase, DispatchSpec, NatsSpec, RedisSpec, ResourceRequirements,
    ResourceSpec, StorageSpec, Toleration, XervCluster, XervClusterSpec, XervClusterStatus,
};
pub use federated_pipeline::{
    AutoscalingConfig, ClusterOverride, ClusterPipelineStatus, ConfigMapRef, FederatedPipeline,
    FederatedPipelineCondition, FederatedPipelineSpec, FederatedPipelineStatus,
    GitSource as FederatedGitSource, PipelineDefinition, PipelineResources,
    PipelineSource as FederatedPipelineSource, PlacementConfig, RolloutConfig, RolloutStrategy,
    SyncStatus, TriggerConfig, TriggerOverride,
};
pub use federation::{
    ClusterMemberStatus, ConflictResolution, FederationCondition, FederationMember,
    FederationPhase, HealthCheckConfig, RoutingConfig, RoutingRule, RoutingStrategy,
    SecurityConfig, SyncConfig, XervFederation, XervFederationSpec, XervFederationStatus,
};
pub use pipeline::{
    GitSource, PipelineCondition, PipelinePhase, PipelineSource, TriggerSpec, XervPipeline,
    XervPipelineSpec, XervPipelineStatus,
};

/// Type alias for backward compatibility with pipeline triggers.
pub type PipelineTrigger = TriggerSpec;
