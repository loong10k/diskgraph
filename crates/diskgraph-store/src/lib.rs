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
mod control_retention;
mod control_store;
mod directory_aggregates;
mod directory_budget_query;
#[cfg(test)]
mod directory_budget_tests;
mod directory_queries;
mod evidence_queries;
mod execution_codec;
mod full_node_row;
mod graph_migrations;
mod graph_validation;
mod history_metadata_query;
mod history_node_cursor;
mod history_queries;
mod intent_state;
mod job_kind;
mod job_record;
mod job_state;
mod job_store;
#[cfg(test)]
mod locator_budget_tests;
mod native_locator_migration;
mod native_locator_query;
#[cfg(test)]
mod native_locator_tests;
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
mod policy_store;
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
mod revision_edge_cursor;
mod revision_evidence_reader;
#[cfg(test)]
mod revision_evidence_tests;
mod revision_queries;
mod revision_record;
mod revision_relation_page_queries;
mod revision_relation_queries;
mod revision_source_validation;
mod revision_writer;
mod scan_staging_store;
mod scope_record;
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
