//! DTOs for the HTTP layer.
//!
//! Responses are the kernel entities serialized directly — one definition,
//! shared with the services layer and the `admin-rs` client.

pub mod healthcheck;

pub use admin_kernel::BoxedError;
pub use admin_kernel::entities::{
    DockerStatus, ProxydState, ProxydStatus, RouteCounts, RouteView, RoutesStatus, ServiceStatus,
    StatusReport, TerminatedView,
};
