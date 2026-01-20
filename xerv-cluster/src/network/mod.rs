//! Network layer for Raft RPC communication.
//!
//! This module implements the gRPC-based network transport for Raft messages
//! using tonic.

mod client;
mod server;

pub use client::NetworkClient;
pub use server::RaftServer;
