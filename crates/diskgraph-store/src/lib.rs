//! DiskGraph 存储入口：仅声明模块与稳定 API 导出。

mod approval;
mod approval_row;
mod approval_store;
mod authorization_generation;
mod candidate_query;
mod candidate_selection;
mod collector_batch_writer;
mod collector_membership_migration;
#[cfg(test)]
mod collector_migration_integrity_tests;
mod collector_protocol;
mod collector_publication;
#[cfg(test)]
mod collector_publication_tests;
mod collector_store;
mod control_codec;
mod control_read_budget;
#[cfg(test)]
mod control_read_budget_tests;
mod control_retention;
mod control_store;
#[cfg(test)]
mod control_write_boundary_interrupt_tests;
#[cfg(test)]
mod control_write_boundary_retry_tests;
mod control_write_deadline;
#[cfg(test)]
mod control_write_deadline_probe_tests;
#[cfg(test)]
mod control_write_deadline_tests;
mod directory_aggregates;
mod directory_budget_query;
#[cfg(test)]
mod directory_budget_tests;
mod directory_queries;
mod evidence_queries;
mod execution_codec;
mod full_node_row;
mod git_collector_base;
mod git_collector_batch_validation;
mod git_collector_publication;
#[cfg(test)]
mod git_collector_publication_tests;
mod git_collector_selection;
#[cfg(test)]
mod git_job_authorization_tests;
#[cfg(test)]
mod git_job_contract_tests;
mod git_job_failure_store;
#[cfg(test)]
mod git_job_failure_tests;
mod git_job_input_codec;
#[cfg(test)]
mod git_job_input_tests;
mod git_job_migration;
#[cfg(test)]
mod git_job_migration_tests;
mod git_job_recovery;
mod git_job_store;
#[cfg(test)]
mod git_job_test_fixtures;
#[cfg(test)]
mod git_legacy_terminal_tests;
#[cfg(test)]
mod git_owned_base_tests;
#[cfg(test)]
mod git_raw_allocation_tests;
#[cfg(test)]
mod git_selection_integrity_tests;
mod graph_migrations;
mod graph_validation;
mod history_metadata_query;
mod history_node_cursor;
mod history_queries;
mod intent_state;
mod job_authority_claim;
#[cfg(test)]
mod job_authority_decode_tests;
mod job_authority_gate;
#[cfg(test)]
mod job_authority_migration_tests;
mod job_authority_store;
#[cfg(test)]
mod job_authority_tests;
mod job_cancel_generation;
#[cfg(test)]
mod job_cancel_generation_tests;
mod job_kind;
#[cfg(test)]
mod job_publication_check_tests;
mod job_queue_query;
#[cfg(test)]
mod job_queue_query_tests;
mod job_receipt_migration;
mod job_receipt_query;
mod job_record;
mod job_state;
mod job_store;
#[cfg(test)]
mod locator_budget_tests;
mod metadata_read_cost;
mod migration_backup;
mod native_locator_migration;
mod native_locator_query;
#[cfg(test)]
mod native_locator_tests;
mod navigation_node;
mod navigation_query;
#[cfg(test)]
mod navigation_query_tests;
mod node_budget_query;
mod node_codec;
mod node_queries;
mod node_row;
mod operation;
mod operation_item;
mod operation_item_result;
mod operation_state;
mod operation_store;
mod plan;
mod plan_item;
mod plan_state;
mod plan_store;
mod policy_permission;
#[cfg(test)]
mod policy_permission_tests;
mod policy_store;
#[cfg(test)]
mod process_job_protocol_contract_tests;
#[cfg(test)]
mod reader_admission_tests;
#[cfg(test)]
mod reader_prepare_benchmark;
mod recovery_entry;
mod recovery_rule;
mod recovery_state;
mod recovery_store;
#[cfg(test)]
mod relation_budget_tests;
mod relation_membership_index;
mod relation_page_queries;
mod relation_queries;
mod resource_node_identity;
mod result;
mod retention_store;
mod revision_access_audit;
mod revision_access_migration;
mod revision_edge_cursor;
mod revision_evidence_reader;
#[cfg(test)]
mod revision_evidence_tests;
mod revision_queries;
mod revision_record;
mod revision_relation_page_queries;
mod revision_relation_queries;
mod revision_source_validation;
mod revision_target_query;
#[cfg(test)]
mod revision_target_query_tests;
mod revision_writer;
mod scan_staging_store;
mod scope_preparation_store;
mod scope_record;
mod scope_registration_transaction;
#[cfg(test)]
mod scope_registration_transaction_tests;
mod scope_store;
mod search_queries;
mod snapshot_queries;
mod snapshot_writer;
mod sqlite_snapshot_store;
mod staging_locator_validation;
mod staging_node_encoding;
mod store_error;
mod stored_node_locator;
mod tree_budget_query;
#[cfg(test)]
mod tree_budget_tests;
mod tree_row;

#[cfg(test)]
mod child_aggregate_benchmark;
#[cfg(test)]
mod child_aggregate_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod execution_tests;
#[cfg(test)]
mod search_keyset_tests;
#[cfg(test)]
mod tests;

pub use approval::Approval;
pub use candidate_selection::CandidateSelection;
pub use control_store::ControlStore;
pub use history_node_cursor::HistoryNodeCursor;
pub use intent_state::IntentState;
pub use job_kind::JobKind;
pub use job_record::JobRecord;
pub use job_state::JobState;
pub use navigation_node::NavigationNode;
pub use operation::Operation;
pub use operation_item::OperationItem;
pub use operation_item_result::OperationItemResult;
pub use operation_state::OperationState;
pub use plan::Plan;
pub use plan_item::PlanItem;
pub use plan_state::PlanState;
pub use recovery_entry::RecoveryEntry;
pub use recovery_rule::RecoveryRule;
pub use recovery_state::RecoveryState;
pub use result::Result;
pub use revision_record::RevisionRecord;
pub use scope_record::ScopeRecord;
pub use sqlite_snapshot_store::{SUPPORTED_SCHEMA_VERSION, SqliteSnapshotStore};
pub use store_error::StoreError;
pub use tree_row::TreeRow;

pub use revision_evidence_reader::RevisionEvidenceReader;

#[cfg(test)]
mod revision_candidate_tests;

#[cfg(test)]
mod collector_protocol_tests;

#[cfg(test)]
mod revision_history_budget_tests;

pub use staging_node_encoding::staging_node_encoded_cost;
pub use stored_node_locator::StoredNodeLocator;

#[cfg(test)]
mod windows_observation_tests;

mod stored_windows_observation;
mod windows_observation_codec;
mod windows_observation_migration;
mod windows_observation_query;
pub use staging_node_encoding::staging_observed_node_encoded_cost;
pub use stored_windows_observation::StoredWindowsObservation;

#[cfg(test)]
mod windows_observation_fixtures;
#[cfg(test)]
mod windows_observation_publication_tests;

#[cfg(test)]
mod native_locator_point_query_tests;

mod process_collector_base;
mod process_collector_batch_validation;
mod process_collector_publication;
mod process_collector_selection;
mod process_collector_target;
mod process_entity_identity;
mod process_fence_admission;
mod process_fence_authorization;
mod process_job_enqueue_until;
mod process_job_failure_store;
mod process_job_input_codec;
mod process_job_limits_view;
mod process_job_migration;
mod process_job_recovery;
mod process_job_store;
mod process_preparation_store;
mod process_receipt_migration;
mod process_receipt_query;
mod stored_unix_observation;
mod unix_observation_query;
mod unix_observation_staging;
pub use stored_unix_observation::StoredUnixObservation;

#[cfg(test)]
mod process_collector_publication_tests;
#[cfg(test)]
mod process_fence_admission_tests;
#[cfg(test)]
mod process_job_authorization_tests;
#[cfg(test)]
mod process_job_enqueue_deadline_tests;
#[cfg(test)]
mod process_job_enqueue_fixture;
#[cfg(test)]
mod process_job_recovery_tests;
#[cfg(test)]
mod process_job_test_fixtures;
#[cfg(test)]
mod process_preparation_projection_tests;
#[cfg(test)]
mod process_raw_allocation_tests;
#[cfg(test)]
mod process_unix_observation_tests;
#[cfg(test)]
mod process_unix_staging_query_tests;
#[cfg(test)]
mod scope_native_projection_tests;

#[cfg(test)]
mod withdrawal_capability_tests;
#[cfg(all(test, windows))]
mod withdrawal_native_identity_tests;
#[cfg(all(test, windows))]
mod withdrawal_registry_tests;
#[cfg(all(test, windows))]
mod withdrawal_test_fixture;
#[cfg(all(test, windows))]
mod withdrawal_watch_namespace_tests;
#[cfg(all(test, windows))]
mod withdrawal_watch_tests;

mod authorization_withdrawal_status;

mod authorization_withdrawal_watch;

mod control_database_identity;

mod control_store_incarnation;

mod withdrawal_entry;

mod withdrawal_registry;

mod withdrawal_store;
pub use authorization_withdrawal_status::AuthorizationWithdrawalStatus;
pub use authorization_withdrawal_watch::AuthorizationWithdrawalWatch;
#[cfg(test)]
mod withdrawal_publish_hook;
#[cfg(all(test, windows))]
mod withdrawal_publish_order_tests;

mod staging_unix_observation_encoding;
pub use staging_unix_observation_encoding::staging_unix_observation_encoded_cost;
#[cfg(test)]
mod unix_staging_cost_tests;

#[cfg(test)]
mod scan_receipt_protocol_tests;

mod scan_receipt_migration;

mod scan_publication_receipt;
pub use scan_publication_receipt::ScanPublicationReceipt;
mod scan_receipt_query;

mod scan_job_recovery;
