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
pub mod live_evidence;
mod native_locator;
pub mod native_process;
mod policy_service;
#[cfg(test)]
mod process_entry_budget_tests;
mod process_evidence_admission;
mod process_evidence_entry;
mod process_evidence_target;
mod process_job_status;
mod queries;
mod relation_access;
mod relation_queries;
mod relation_request;
#[cfg(test)]
mod relation_request_tests;
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
mod scope_service;
mod scoped_content;
#[cfg(not(windows))]
mod scoped_file;
mod snapshot_retention;
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
pub use verify_limits::VerifyLimits;
