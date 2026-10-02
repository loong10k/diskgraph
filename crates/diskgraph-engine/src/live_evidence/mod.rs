//! live_evidence 的稳定模块路径，仅声明与明确重导出。

mod fs_event;
mod fs_event_kind;
#[cfg(test)]
mod git_cleanup_tests;
mod git_command_context;
#[cfg(all(test, unix))]
mod git_command_context_tests;
mod git_config_policy;
mod git_configuration;
mod git_directory_lease;
#[cfg(windows)]
mod git_directory_security;
#[cfg(test)]
mod git_directory_tests;
mod git_directory_version;
mod git_executable;
#[cfg(test)]
mod git_execution_policy_tests;
mod git_index_layout;
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
mod git_output;
mod git_private_allocation;
mod git_private_capacity;
#[cfg(test)]
mod git_private_capacity_tests;
mod git_private_directory;
#[cfg(test)]
mod git_private_integrity_tests;
mod git_references;
mod git_reflog_file;
#[cfg(test)]
mod git_resource_tests;
mod git_sample;
#[cfg(test)]
mod git_semantics_tests;
mod git_stash;
#[cfg(test)]
mod git_stash_tests;
mod git_system_configuration;
#[cfg(test)]
mod git_system_configuration_tests;
mod git_tool_path;
#[cfg(test)]
mod git_tool_path_tests;
mod git_usage;
mod git_view;
#[cfg(target_os = "macos")]
mod macos_probe_group;
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
