//! DiskGraph Engine 入口：仅声明模块并保留稳定公开路径。

mod collectors;
mod compare_row;
mod comparison_report;
pub mod content;
mod contextual_engine_error;
mod control_access;
mod engine;
mod engine_capacity;
mod engine_config;
mod engine_error;
mod engine_startup;
mod explanation;
#[cfg(test)]
mod git_evidence_admission_tests;
mod git_evidence_batch;
mod git_evidence_entry;
mod git_evidence_execution;
#[cfg(test)]
mod git_evidence_execution_tests;
mod git_evidence_failure;
#[cfg(test)]
mod git_evidence_fixture;
#[cfg(test)]
mod git_evidence_publication_tests;
mod git_evidence_target;
#[cfg(test)]
mod git_job_cancellation_tests;
mod git_job_status;
#[cfg(test)]
mod git_job_status_tests;
#[cfg(test)]
mod git_late_enqueue_tests;
mod history_namespace;
mod history_node_lookup;
mod history_request;
mod job_authorization;
#[cfg(test)]
mod job_authorization_tests;
mod job_cancellation_guard;
#[cfg(test)]
mod job_cancellation_guard_tests;
mod job_execution;
mod job_execution_stop_reason;
#[cfg(test)]
mod job_keeper_unwind_tests;
mod job_request_cancel_bridge;
#[cfg(test)]
mod job_stop_cause_fixture;
#[cfg(test)]
mod job_stop_cause_hooks;
#[cfg(test)]
mod job_stop_cause_outcome_tests;
#[cfg(test)]
mod job_stop_cause_tests;
#[cfg(test)]
mod job_stop_conflict_tests;
#[cfg(test)]
mod job_stop_registration_tests;
#[cfg(test)]
mod job_stop_signals_tests;
pub mod live_evidence;
mod native_child;
mod native_locator;
pub mod native_process;
mod policy_service;
#[cfg(test)]
mod process_entry_budget_tests;
mod process_evidence_admission;
#[cfg(target_os = "linux")]
mod process_evidence_batch;
mod process_evidence_entry;
#[cfg(target_os = "linux")]
mod process_evidence_execution;
mod process_evidence_target;
#[cfg(any(target_os = "linux", test))]
mod process_execution_fence;
#[cfg(all(test, target_os = "linux"))]
mod process_execution_fixture;
#[cfg(target_os = "linux")]
mod process_execution_target;
#[cfg(all(test, target_os = "linux"))]
mod process_execution_tests;
#[cfg(test)]
mod process_fence_priority_tests;
mod process_job_status;
#[cfg(any(target_os = "linux", test))]
mod process_native_error;
#[cfg(all(test, target_os = "linux"))]
mod process_publication_tests;
mod queries;
mod relation_access;
mod relation_queries;
mod relation_request;
#[cfg(test)]
mod relation_request_tests;
mod request_withdrawal_witness;
mod revision_authorization;
mod revision_comparison;
mod revision_display_completion;
#[cfg(test)]
mod revision_evidence_tests;
mod revision_growth;
mod revision_history;
mod revision_locator;
#[cfg(test)]
mod revision_locator_tests;
mod revision_queries;
mod revision_reader;
mod revision_windows_observation;
#[cfg(test)]
mod revision_windows_observation_tests;
mod runner;
mod scan_execution;
mod scan_image_identity;
mod scan_jobs;
#[cfg(test)]
mod scan_locator_tests;
mod scan_node_locator;
mod scan_observation_guard;
#[cfg(test)]
mod scan_observation_tests;
mod scan_progress_guard;
#[cfg(test)]
mod scan_publication_tests;
mod scan_worker_host_config;
mod scan_worker_installation;
mod scope_service;
mod scoped_content;
#[cfg(not(windows))]
mod scoped_file;
mod snapshot_retention;
#[cfg(test)]
mod status_contention_tests;
mod sync_plan;
#[cfg(test)]
mod tests;
mod tree_queries;
pub mod verify;
mod verify_limits;
#[cfg(windows)]
mod windows_file_state;
#[cfg(windows)]
mod windows_native_open;
#[cfg(windows)]
mod windows_native_scan_root;
#[cfg(all(test, windows))]
mod windows_native_scan_tests;
#[cfg(windows)]
mod windows_path_plan;
#[cfg(windows)]
mod windows_scoped_file;

pub use collectors::{
    COLLECTOR_ID, COLLECTOR_VERSION, ProjectBatch, RULE_VERSION, collect_projects,
};
pub use compare_row::CompareRow;
pub use comparison_report::ComparisonReport;
pub use contextual_engine_error::ContextualEngineError;
pub use engine::Engine;
pub use engine_config::EngineConfig;
pub use engine_error::EngineError;
pub use explanation::Explanation;
pub use policy_service::admin_scope;
pub use queries::{
    ExploreSummary, ImpactEntry, ImpactResult, Propagation, cursor_context, explore, impact,
    impact_bounded, impact_bounded_with_neighbors, impact_propagation, incompatibility_name,
    search_nodes,
};
pub use revision_display_completion::RevisionDisplayCompletion;
pub use revision_growth::RevisionGrowth;
pub use runner::JobRunner;
pub use scan_worker_host_config::ScanWorkerHostConfig;
pub use scan_worker_installation::ScanWorkerInstallation;
pub use verify_limits::VerifyLimits;

#[cfg(test)]
mod engine_error_cleanup_tests;
