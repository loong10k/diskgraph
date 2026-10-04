//! UniFFI 只读绑定入口：模块声明、明确导出与必需脚手架。

mod api_response;
mod api_result;
mod job_authorization;
mod job_state;
mod native_admission;
#[cfg(test)]
mod native_admission_tests;
mod native_growth;
#[cfg(test)]
mod native_growth_tests;
mod native_job_entry;
mod native_jobs;
mod native_join_task;
mod native_lifecycle;
#[cfg(test)]
mod native_lifecycle_tests;
mod native_listing;
#[cfg(test)]
mod native_listing_tests;
#[cfg(test)]
mod native_manager_tests;
mod native_realm;
mod native_reply;
#[cfg(test)]
mod native_revision_candidate_tests;
#[cfg(test)]
mod native_runner_exit_tests;
mod native_runner_guard;
#[cfg(test)]
mod native_runner_signal_tests;
#[cfg(test)]
mod native_scan_gate;
#[cfg(test)]
mod native_scoped_owner_tests;
mod native_service;
mod native_service_error;
mod native_service_host_guard;
mod native_service_owner;
#[cfg(test)]
mod native_service_owner_tests;
mod native_worker;
#[cfg(test)]
mod native_worker_exit_barrier;
#[cfg(test)]
mod native_worker_exit_tests;
mod scan_coordinator;
pub(crate) use api_response::{bounded_limit, response};
pub(crate) use api_result::ApiResult;
pub(crate) use job_authorization::JobAuthorization;
pub(crate) use job_state::JobState;
#[cfg(all(test, unix))]
pub(crate) use native_realm::path_digest;
#[cfg(test)]
pub(crate) use native_realm::realm_dir_for_database;
pub(crate) use native_realm::{local_principal, open_engine};
pub use native_service::NativeService;
pub use native_service_error::NativeServiceError;
pub use native_service_owner::NativeServiceOwner;
pub(crate) use scan_coordinator::{run_scan_on_engine, run_scan_with_cancel, spawn_job};
#[cfg(test)]
mod async_tests;
#[cfg(test)]
mod authorization_tests;
#[cfg(test)]
mod native_service_tests;
#[cfg(test)]
mod tests;

// UniFFI 的词法路径参与旧绑定校验；只包含这两个真实实现文件。
include!("api_exports.rs");
include!("job_handle.rs");

uniffi::setup_scaffolding!();

#[cfg(test)]
mod native_growth_eligibility_tests;

#[cfg(all(test, any(unix, windows)))]
mod native_growth_scope_fixture;
#[cfg(all(test, any(unix, windows)))]
mod native_growth_scope_tests;
