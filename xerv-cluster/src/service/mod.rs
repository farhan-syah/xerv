//! External client service for interacting with the cluster.
//!
//! This module provides the ClusterService gRPC implementation that external
//! clients use to execute commands, query status, and manage membership.

mod cluster;

pub use cluster::ClusterServiceImpl;
