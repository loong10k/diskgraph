//! live_evidence 的稳定模块路径，仅声明与明确重导出。

#[cfg(test)]
mod evidence_probe_completion_tests;
#[cfg(all(test, unix))]
mod evidence_probe_failure_tests;
mod evidence_probe_session;
#[cfg(test)]
mod evidence_probe_session_tests;
mod fs_event;
mod fs_event_kind;
#[cfg(test)]
mod git_callback_tests;
#[cfg(test)]
mod git_cleanup_tests;
mod git_command_context;
#[cfg(all(test, unix))]
mod git_command_context_tests;
mod git_config_policy;
#[cfg(test)]
mod git_config_policy_tests;
mod git_configuration;
mod git_directory_lease;
#[cfg(windows)]
mod git_directory_security;
#[cfg(test)]
mod git_directory_tests;
mod git_directory_version;
mod git_executable;
#[cfg(test)]
mod git_executable_native_tests;
#[cfg(test)]
mod git_execution_policy_tests;
mod git_index_layout;
mod git_indexed_directory;
#[cfg(test)]
mod git_indexed_identity_tests;
#[cfg(test)]
mod git_indexed_windows_identity_tests;
#[cfg(test)]
mod git_input_read_tests;
#[cfg(test)]
mod git_isolation_fixture;
#[cfg(test)]
mod git_isolation_semantics_tests;
#[cfg(test)]
mod git_isolation_tests;
mod git_metadata_budget;
mod git_metadata_directory;
mod git_metadata_file;
#[cfg(test)]
mod git_metadata_tests;
mod git_metadata_tree;
mod git_metadata_version;
mod git_native_path;
#[cfg(test)]
mod git_native_path_tests;
mod git_object_database;
#[cfg(test)]
mod git_object_database_tests;
#[cfg(test)]
mod git_object_resources_tests;
mod git_output;
mod git_private_allocation;
mod git_private_capacity;
#[cfg(test)]
mod git_private_capacity_tests;
mod git_private_directory;
#[cfg(test)]
mod git_private_integrity_tests;
mod git_product_error;
#[cfg(test)]
mod git_product_tests;
mod git_references;
mod git_reflog_file;
#[cfg(test)]
mod git_resource_tests;
mod git_sample;
mod git_scope_boundary;
#[cfg(test)]
mod git_scope_boundary_tests;
#[cfg(test)]
mod git_scoped_fixture;
#[cfg(test)]
mod git_scoped_semantics_tests;
#[cfg(test)]
mod git_semantics_tests;
mod git_source_directory;
mod git_source_file;
#[cfg(unix)]
mod git_source_unix;
#[cfg(windows)]
mod git_source_windows;
#[cfg(all(test, windows))]
mod git_source_windows_diagnostic;
#[cfg(all(test, windows))]
mod git_source_windows_phase;
mod git_stash;
#[cfg(test)]
mod git_stash_tests;
#[cfg(test)]
mod git_status_output_tests;
mod git_system_configuration;
#[cfg(test)]
mod git_system_configuration_tests;
mod git_tool_path;
#[cfg(test)]
mod git_tool_path_tests;
mod git_usage;
mod git_view;
#[cfg(test)]
mod git_view_race_tests;
mod git_view_sources;
mod git_worktree_capture;
#[cfg(test)]
mod git_worktree_capture_tests;
#[cfg(all(test, target_os = "macos"))]
mod macos_probe_tests;
mod probe_budget;
mod probe_execution;
mod probe_failure;
#[cfg(test)]
mod probe_isolation_tests;
mod probe_limits;
mod probe_output;
#[cfg(test)]
mod probe_tests;
#[cfg(windows)]
mod probe_windows;
#[cfg(test)]
mod process_allocation_tests;
mod process_holder;
mod process_output;
#[cfg(test)]
mod process_output_tests;
mod process_usage;
mod sampling_clock;
#[cfg(test)]
mod tests;
#[cfg(unix)]
mod unix_probe_child;
mod usage_coverage;
mod usage_sample;
mod watch_poll;
mod watch_report;
mod watch_snapshot;

pub use evidence_probe_session::EvidenceProbeSession;
pub use fs_event::FsEvent;
pub use fs_event_kind::FsEventKind;
pub use git_sample::GitSample;
pub use git_usage::{sample_git, sample_git_bounded};
pub use probe_limits::ProbeLimits;
pub use process_holder::ProcessHolder;
pub use process_usage::{sample_process_usage, sample_process_usage_bounded};
pub use usage_coverage::UsageCoverage;
pub use usage_sample::UsageSample;
pub use watch_poll::poll_changes;
pub use watch_report::WatchReport;
pub use watch_snapshot::WatchSnapshot;

pub(crate) use git_indexed_directory::GitIndexedDirectory;
