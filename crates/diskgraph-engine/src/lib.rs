//! DiskGraph Engine 入口：仅声明模块并保留稳定公开路径。

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod native_scan_engine_fixture;

#[cfg(test)]
mod admin_policy_lookup_tests;
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
mod initial_revision_authorization;
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
#[cfg(windows)]
mod probe_directory_binding;
#[cfg(windows)]
mod probe_host;
#[cfg(windows)]
mod probe_recovery;
#[cfg(windows)]
mod probe_resource_pool;
#[cfg(windows)]
mod probe_resource_slot;
#[cfg(windows)]
mod probe_session_lease;
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
#[cfg(test)]
mod relation_query_diagnostics_tests;
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
mod revision_reader_authorization;
mod revision_root_reconciliation;
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
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_error_projection;
mod scan_worker_host;
#[cfg(test)]
mod scan_worker_host_admission_tests;
mod scan_worker_host_config;
mod scan_worker_installation;
mod scan_worker_owner_slot;
mod scan_worker_recovery;
#[cfg(all(test, target_os = "linux"))]
mod scan_worker_recovery_fixture;
#[cfg(all(test, target_os = "linux"))]
mod scan_worker_recovery_tests;
mod scan_worker_registry;
#[cfg(test)]
mod scan_worker_registry_tests;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_remote_error;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
mod scan_worker_reservation;
mod scan_worker_runtime;
mod scan_worker_runtime_budget;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod scan_worker_runtime_hooks;
mod scan_worker_settings;
#[cfg(test)]
mod scan_worker_settings_tests;
mod scope_service;
mod scoped_content;
#[cfg(not(windows))]
mod scoped_file;
mod snapshot_retention;
#[cfg(test)]
mod status_contention_tests;
mod sync_plan;
#[cfg(test)]
mod terminal_capability_tests;
mod terminal_revision_authorization;
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
mod windows_scan_image_binding;
#[cfg(windows)]
mod windows_scan_image_lease;
#[cfg(all(test, windows))]
mod windows_scan_image_lease_tests;
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
#[cfg(windows)]
pub use probe_host::ProbeHost;
#[cfg(windows)]
pub use probe_recovery::ProbeRecovery;
pub use queries::{
    ExploreSummary, ImpactEntry, ImpactResult, Propagation, cursor_context, explore, impact,
    impact_bounded, impact_bounded_with_neighbors, impact_propagation, incompatibility_name,
    search_nodes,
};
pub use revision_display_completion::RevisionDisplayCompletion;
pub use revision_growth::RevisionGrowth;
pub use runner::JobRunner;
pub use scan_worker_host::ScanWorkerHost;
pub use scan_worker_host_config::ScanWorkerHostConfig;
pub use scan_worker_installation::ScanWorkerInstallation;
pub use scan_worker_recovery::ScanWorkerRecovery;
pub use scan_worker_runtime_budget::ScanWorkerRuntimeBudget;
pub use scan_worker_settings::ScanWorkerSettings;
pub use verify_limits::VerifyLimits;

#[cfg(test)]
mod engine_error_cleanup_tests;

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_budget_classification_tests;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_child;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_driver;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_driver_additional_tests;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_driver_test_support;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_driver_tests;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_failure;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_input;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_input_budget_tests;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_input_buffer;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_output;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod scan_worker_owned_failure;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_owned_failure_test_support;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod scan_worker_owned_failure_tests;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod scan_worker_slot_generation_tests;

#[cfg(target_os = "macos")]
mod macos_filesystem_state;
#[cfg(all(test, target_os = "macos"))]
mod macos_filesystem_state_tests;
#[cfg(target_os = "macos")]
mod macos_install_receipt;
#[cfg(all(test, target_os = "macos"))]
mod macos_install_receipt_tests;
#[cfg(target_os = "macos")]
mod macos_installation_claims;
#[cfg(target_os = "macos")]
mod macos_installation_lease;
#[cfg(all(test, target_os = "macos"))]
mod macos_installation_lease_tests;
#[cfg(target_os = "macos")]
mod macos_installation_trust;
#[cfg(target_os = "macos")]
mod macos_open_namespace;
#[cfg(target_os = "macos")]
pub use macos_installation_trust::MacosInstallationTrust;

#[cfg(target_os = "macos")]
mod macos_load_policy;
#[cfg(all(test, target_os = "macos"))]
mod macos_load_policy_tests;

#[cfg(target_os = "macos")]
mod macos_protected_document;
#[cfg(all(test, target_os = "macos"))]
mod macos_protected_document_tests;

#[cfg(target_os = "macos")]
mod macos_host_settings;
#[cfg(all(test, target_os = "macos"))]
mod macos_host_settings_tests;

#[cfg(all(test, target_os = "macos"))]
mod macos_host_admission_tests;

#[cfg(target_os = "macos")]
mod macos_installation_lock;
#[cfg(all(test, target_os = "macos"))]
mod macos_installation_lock_tests;

#[cfg(target_os = "macos")]
mod macos_spawn_permit;

#[cfg(target_os = "macos")]
mod macos_epoch_floor;
#[cfg(all(test, target_os = "macos"))]
mod macos_epoch_floor_tests;

#[cfg(target_os = "macos")]
mod macos_installation_publisher;
#[cfg(target_os = "macos")]
pub use macos_installation_publisher::MacosInstallationPublisher;

#[cfg(target_os = "macos")]
mod macos_installation_files;

#[cfg(all(test, target_os = "macos"))]
mod macos_installation_publisher_tests;

#[cfg(target_os = "macos")]
mod macos_installation_bootstrap;
#[cfg(all(test, target_os = "macos"))]
mod macos_installation_bootstrap_tests;

#[cfg(all(test, target_os = "macos"))]
mod macos_installation_root_fixture_tests;

#[cfg(all(test, target_os = "macos"))]
mod macos_installed_worker_fixture_tests;

#[cfg(all(test, target_os = "macos", feature = "macos_native_scan_candidate"))]
mod macos_engine_scan_fixture_tests;

#[cfg(all(test, windows))]
mod windows_scan_image_mapping_tests;

#[cfg(windows)]
mod windows_scan_launcher;
#[cfg(all(test, windows))]
mod windows_scan_runtime_tests;

#[cfg(all(test, windows))]
mod probe_pool_cleanup_fault;

pub mod recovery_control;

pub mod recovery_slot;

#[cfg(test)]
mod admission_seal_tests;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod supervisor_owner;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod supervisor_parts;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod supervisor_recovery_error;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub use supervisor_owner::SupervisorOwner;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub use supervisor_parts::SupervisorParts;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub use supervisor_recovery_error::SupervisorRecoveryError;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod supervisor_binding_tests;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub mod native_deadline;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod trusted_local_recovery_domain;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod trusted_local_recovery_home;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use trusted_local_recovery_domain::TrustedLocalRecoveryDomain;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod trusted_local_recovery_domain_tests;

#[cfg(test)]
mod scope_authorization_tests;

mod authority_expiry;

mod live_permission;

#[cfg(target_os = "linux")]
mod linux_no_recall_open;

#[cfg(all(test, target_os = "linux"))]
mod linux_no_recall_open_tests;

#[cfg(test)]
mod authority_expiry_clock;

mod job_status_read;

#[cfg(test)]
mod revision_reader_lock_gap_tests;

#[cfg(test)]
mod history_relation_withdrawal_tests;

mod request_metadata_withdrawals;

mod scan_publication_recovery;

#[cfg(test)]
mod scan_receipt_recovery_tests;
