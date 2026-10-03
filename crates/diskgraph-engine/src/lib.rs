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
mod history_node_lookup;
mod history_request;
pub mod live_evidence;
mod native_locator;
mod policy_service;
mod queries;
mod relation_access;
mod relation_queries;
mod relation_request;
#[cfg(test)]
mod relation_request_tests;
mod revision_authorization;
mod revision_comparison;
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
pub use revision_growth::RevisionGrowth;
pub use runner::JobRunner;
pub use verify_limits::VerifyLimits;
