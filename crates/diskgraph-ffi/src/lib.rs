//! UniFFI 只读绑定入口：模块声明、明确导出与必需脚手架。

mod api_response;
mod api_result;
mod job_authorization;
mod job_state;
mod native_growth;
#[cfg(test)]
mod native_growth_tests;
mod native_jobs;
mod native_listing;
#[cfg(test)]
mod native_listing_tests;
mod native_realm;
mod native_reply;
#[cfg(test)]
mod native_revision_candidate_tests;
#[cfg(test)]
mod native_scan_gate;
mod native_service;
mod native_service_error;
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
