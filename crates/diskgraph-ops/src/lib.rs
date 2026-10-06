//! Reversible file operations (P5). This crate turns "the agent wants to move
//! these files" into an immutable, digest-bound, human-approvable plan, and
//! only executes it after a trusted approval and a fresh revalidation.
//!
//! The layering is deliberate and matches the spec's separation:
//! - [`PlanBuilder`] resolves a request into concrete objects. It mutates no
//!   source file; it reads source evidence and persists a control-store plan.
//! - [`ApprovalIssuer`] mints the approval an external trusted surface
//!   (PruneX review UI, an admin console) would present. The agent can never
//!   mint one for itself.
//! - [`Executor`] is the only place a file moves, and only for a plan whose
//!   digest still matches an unexpired approval, after revalidating every
//!   precondition against the live filesystem.
mod apply_outcome;
mod apply_request;
mod approval_issuer;
mod atomic_publish;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod bound_path;
mod cross_volume_copy;
mod cross_volume_staging;
pub mod docker;
mod executor;
mod executor_apply;
mod executor_paths;
mod executor_perform;
mod executor_transfer;
mod executor_validation;
mod fault_point;
mod job_outcome;
mod live_item;
#[cfg(target_os = "macos")]
mod metadata_fidelity;
mod operation_description;
mod operation_queries;
mod operation_view;
mod ops_authorization;
mod ops_error;
mod ops_time;
mod path_codec;
mod path_fault;
mod path_revalidation;
mod plan_builder;
mod plan_digest;
mod plan_request;
mod raw_path;
mod resolved;
mod scope_refresh;
mod side;
mod source_evidence;
pub mod specialist;
mod step_result;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod verified_source;
mod volume_capacity;
mod volume_identity;
mod volume_report;
pub use apply_outcome::ApplyOutcome;
pub use apply_request::ApplyRequest;
pub use approval_issuer::ApprovalIssuer;
pub use docker::{DockerInventory, DockerObject, UsageCheck, VM_CAVEAT};
pub use executor::Executor;
pub use fault_point::FaultPoint;
pub use job_outcome::JobOutcome;
pub use operation_queries::{cancel_operation, list_operations, show_operation};
pub use operation_view::OperationView;
pub use ops_error::OpsError;
pub use path_fault::PathFault;
pub use path_revalidation::revalidate_below;
pub use plan_builder::PlanBuilder;
pub use plan_digest::plan_digest;
pub use scope_refresh::refresh_scope_after_operation;
pub use side::Side;
pub use specialist::{
    AdapterRegistry, AdapterStatus, CARGO_CLEAN, CleanupInventory, CommandRunner, CommandSpec,
    DOCKER_CLEAN, DOCKER_INVENTORY, InventoryObject, SandboxedRunner, SpecialistVerdict,
};
pub use volume_capacity::{quarantine_retained_bytes, volume_free_bytes};
pub use volume_report::VolumeReport;
#[cfg(all(test, target_os = "linux"))]
mod native_scan_project;
#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
#[cfg(all(test, windows))]
mod unsupported_windows_tests;
