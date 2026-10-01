//! Read-only, platform-neutral disk facts, evidence queries, and the v2
//! contract layer (identity, locators, permissions, catalog, envelope, ports).

mod budget;
mod catalog;
pub mod compare;
mod duplicates;
mod entities;
mod envelope;
mod errors;
mod freshness;
mod ids;
mod locator;
mod model;
mod permissions;
mod ports;
mod query;
mod scan;
pub mod syncplan;
pub mod treemap;

pub use budget::{
    BudgetTracker, CursorContext, CursorRejection, PagingCursor, QueryBudget, TruncationReason,
};
pub use catalog::{CATALOG, CommandAction, CommandSpec, Stage, by_family, by_id};
pub use compare::{Comparison, DifferentReason, Evidence, Summary, Verdict, compare};
pub use duplicates::{ConfirmedSet, ContentRelation, SuspectGroup, confirm_group, suspect_groups};
pub use entities::{
    AssertionKind, CollectorRun, EdgeValidationError, Entity, EntityKind, EvidenceRecord, Polarity,
    Relation, RelationEdge, validate_edges,
};
pub use envelope::{API_VERSION, Envelope, EnvelopeError};
pub use errors::BusinessError;
pub use freshness::{FingerprintMap, FingerprintSource, Freshness, classify, edge_freshness};
pub use ids::{InvalidId, PrincipalId, ResourceRef, RevisionId, ScopeId, ServerId};
pub use locator::{Locator, LocatorDecodeError, LocatorKind};
pub use model::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, FileIdentity, NodeKind,
    ResourceLocator, ScanCoverage, ScanSettings,
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
pub use query::{
    Candidate, Change, Changes, ChildListing, Growth, Incompatibility, NodeExplanation, Page,
    SizeFilter, TreeNode, TreeRenderError, TreeView, render_tree, render_tree_rows,
};
pub use scan::{
    BudgetDecision, BudgetFault, BudgetUsage, CapacityReading, ExcludedPath, ExclusionReason,
    PlaceholderPolicy, RescanComparison, ScanBudget, ScanBudgetStop, ScanExclusions, ScanWindow,
    StorageArea, Watermark, WatermarkVerdict, compare_rescan,
};
pub use syncplan::{CopyReason, SyncAction, SyncMethod, SyncPlan, build_plan as build_sync_plan};
pub use treemap::{Placed, Rect, TextRow, Weighted, human_bytes, render_text, squarify};
