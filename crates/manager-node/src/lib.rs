//! gRPC service implementation for a single TigerBeetle manager node.

/// Generated protobuf types.
pub mod proto {
    #![allow(missing_docs, unreachable_pub, missing_debug_implementations)]
    tonic::include_proto!("tigerbeetle.manager");
}

/// gRPC service implementation.
pub mod grpc_service;

pub use grpc_service::{ManagerNodeService, NodeState};
