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
pub mod live_evidence;
mod native_locator;
mod policy_service;
mod queries;
mod relation_access;
mod relation_queries;
mod revision_authorization;
mod revision_comparison;
mod revision_growth;
mod revision_history;
mod revision_queries;
mod revision_reader;
mod runner;
mod scan_execution;
mod scan_jobs;
mod scan_progress_guard;
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
