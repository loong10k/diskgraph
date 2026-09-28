//! Read-only, platform-neutral disk facts and evidence queries.

mod model;
mod query;

pub use model::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, FileIdentity, NodeKind,
    ResourceLocator, ScanCoverage, ScanSettings,
};
pub use query::{Candidate, Growth, NodeExplanation, Page};
