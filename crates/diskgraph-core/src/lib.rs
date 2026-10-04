//! Read-only, platform-neutral disk facts, evidence queries, and the v2
//! contract layer (identity, locators, permissions, catalog, envelope, ports).

mod budget;
mod catalog;
mod children_cursor;
mod collector_batch;
pub mod compare;
mod duplicates;
mod entities;
mod envelope;
mod errors;
mod freshness;
mod git_evidence_codec;
mod git_evidence_failure;
mod git_evidence_failure_code;
mod git_evidence_failure_phase;
mod git_evidence_job_input;
mod git_evidence_limits;
mod git_evidence_summary;
mod historical_node_size;
#[cfg(test)]
mod historical_size_tests;
mod ids;
mod job_authority_origin;
mod job_publication_receipt;
mod job_request_authority;
mod json_size_writer;
mod locator;
mod locator_encoding;
mod model;
mod permissions;
mod ports;
mod qualified_locator;
mod qualified_locator_error;
#[cfg(test)]
mod qualified_locator_tests;
mod query;
#[cfg(test)]
mod query_budget_tests;
mod query_deadline;
mod query_read_budget;
mod scan;
mod scan_coverage;
pub mod syncplan;
pub mod treemap;
mod windows_file_observation;
mod windows_observation_error;
mod windows_observation_gap;
mod windows_tree_alignment;

pub use windows_file_observation::WindowsFileObservation;
pub use windows_observation_error::WindowsObservationError;
pub use windows_observation_gap::WindowsObservationGap;
pub use windows_tree_alignment::WindowsTreeAlignment;

pub use budget::{
    BudgetTracker, CursorContext, CursorRejection, PagingCursor, QueryBudget, TruncationReason,
};
pub use catalog::{CATALOG, CommandAction, CommandSpec, Stage, by_family, by_id};
pub use children_cursor::ChildrenCursor;
pub use collector_batch::CollectorBatch;
pub use compare::{Comparison, DifferentReason, Evidence, Summary, Verdict, compare};
pub use duplicates::{ConfirmedSet, ContentRelation, SuspectGroup, confirm_group, suspect_groups};
pub use entities::{
    AssertionKind, CollectorRun, EdgeValidationError, Entity, EntityKind, EvidenceRecord, Polarity,
    Relation, RelationEdge, validate_edges,
};
pub use envelope::{API_VERSION, Envelope, EnvelopeError};
pub use errors::BusinessError;
pub use freshness::{FingerprintMap, FingerprintSource, Freshness, classify, edge_freshness};
pub use git_evidence_failure::GitEvidenceFailure;
pub use git_evidence_failure_code::GitEvidenceFailureCode;
pub use git_evidence_failure_phase::GitEvidenceFailurePhase;
pub use git_evidence_job_input::GitEvidenceJobInput;
pub use git_evidence_limits::GitEvidenceLimits;
pub use git_evidence_summary::GitEvidenceSummary;
pub use historical_node_size::{comparable_growth_delta, observed_node_size};
pub use ids::{InvalidId, PrincipalId, ResourceRef, RevisionId, ScopeId, ServerId};
pub use job_authority_origin::JobAuthorityOrigin;
pub use job_publication_receipt::JobPublicationReceipt;
pub use job_request_authority::JobRequestAuthority;
pub use json_size_writer::measure_json_bounded;
pub use locator::{Locator, LocatorDecodeError, LocatorKind};
pub use locator_encoding::LocatorEncoding;
pub use model::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, FileIdentity, NodeKind,
    ResourceLocator, ScanSettings,
};
pub use permissions::{
    Authorizer, Decision, DenyAllAuthorizer, DenyReason, FileActionKind, Grant, Permission,
    PolicyAuthorizer,
};
pub use ports::{
    ApprovalDecision, ApprovalVerifierPort, EvidenceCollectorPort, FileOperatorPort,
    ProviderCapabilities, ProviderKind, ProviderOperation, RefreshSchedulerPort, ResourceProvider,
    SizeCapability, VolumeMeter, capability_decision,
};
pub use qualified_locator::QualifiedLocator;
pub use qualified_locator_error::QualifiedLocatorError;
pub use query::{
    Candidate, Change, Changes, ChildListing, Growth, Incompatibility, NodeExplanation, Page,
    SizeFilter, TreeNode, TreeRenderError, TreeView, render_tree, render_tree_rows,
};
pub use query_deadline::query_deadline;
pub use query_read_budget::QueryReadBudget;
pub use scan::{
    BudgetDecision, BudgetFault, BudgetUsage, CapacityReading, ExcludedPath, ExclusionReason,
    PlaceholderPolicy, RescanComparison, ScanBudget, ScanBudgetStop, ScanExclusions, ScanWindow,
    StorageArea, Watermark, WatermarkVerdict, compare_rescan,
};
pub use scan_coverage::ScanCoverage;
pub use syncplan::{
    CopyReason, ExcludeReason, PlanExclusion, SyncAction, SyncMethod, SyncPlan,
    build_plan as build_sync_plan,
};
pub use treemap::{Placed, Rect, TextRow, Weighted, human_bytes, render_text, squarify};

mod search_cursor;
pub use search_cursor::SearchCursor;

#[cfg(test)]
mod windows_file_observation_tests;

#[cfg(test)]
mod git_evidence_contract_tests;
#[cfg(test)]
mod job_request_authority_tests;
#[cfg(test)]
mod process_protocol_git_compatibility_tests;

mod indexed_file_epoch;
mod process_evidence_codec;
mod process_evidence_failure;
mod process_evidence_failure_code;
mod process_evidence_failure_phase;
mod process_evidence_job_input;
mod process_evidence_limits;
mod process_evidence_summary;
mod process_job_publication_receipt;
mod process_observation_code;
mod process_observation_coverage;
mod process_observation_method;
mod process_startup_identity;
mod unix_file_observation;
mod unix_observation_gap;
pub use indexed_file_epoch::IndexedFileEpoch;
pub use process_evidence_failure::ProcessEvidenceFailure;
pub use process_evidence_failure_code::ProcessEvidenceFailureCode;
pub use process_evidence_failure_phase::ProcessEvidenceFailurePhase;
pub use process_evidence_job_input::ProcessEvidenceJobInput;
pub use process_evidence_limits::ProcessEvidenceLimits;
pub use process_evidence_summary::ProcessEvidenceSummary;
pub use process_job_publication_receipt::ProcessJobPublicationReceipt;
pub use process_observation_code::ProcessObservationCode;
pub use process_observation_coverage::ProcessObservationCoverage;
pub use process_observation_method::ProcessObservationMethod;
pub use process_startup_identity::ProcessStartupIdentity;
pub use unix_file_observation::UnixFileObservation;
pub use unix_observation_gap::UnixObservationGap;

#[cfg(test)]
mod process_evidence_contract_tests;
