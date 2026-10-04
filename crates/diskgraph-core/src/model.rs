use crate::ScanCoverage;
use serde::{Deserialize, Serialize};

/// A platform-specific resource identifier. A display path is not an authorization token.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ResourceLocator {
    NativePath(String),
    DocumentUri(String),
}

/// An inode can aid reconciliation but must not be treated as a permanent identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume_id: String,
    pub file_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Directory,
    File,
    Symlink,
    Other,
}

/// The exact scan behavior needed to compare two snapshots meaningfully.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScanSettings {
    pub apparent_size: bool,
    pub follow_links: bool,
    pub include_hidden: bool,
    pub one_filesystem: bool,
    pub max_depth: Option<usize>,
    pub dedup_hardlinks: bool,
}

/// A single observation of one accessible scope, not a claim about the whole device.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiskSnapshot {
    pub id: String,
    pub root: ResourceLocator,
    pub volume_id: Option<String>,
    pub captured_at_unix_ms: u64,
    pub settings: ScanSettings,
    pub coverage: ScanCoverage,
}

/// Size fields describe observed bytes, not guaranteed reclaimable bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiskNode {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub locator: ResourceLocator,
    pub name: String,
    pub kind: NodeKind,
    pub subtree_bytes: u64,
    pub direct_bytes: u64,
    /// False when the provider could not report a size (denied, unscanned, or
    /// unsupported); `subtree_bytes` must then be read as unknown, not zero.
    pub size_known: bool,
    pub files: u64,
    pub directories: u64,
    pub modified_unix_seconds: Option<i64>,
    pub file_identity: Option<FileIdentity>,
    pub category_hint: Option<String>,
    pub reclaim_hint: Option<String>,
    pub read_error: bool,
}

/// Facts and hypotheses remain separate from permission to delete.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRelation {
    OwnedByApplication,
    OwnedByProject,
    UsedByProcess,
    Rebuildable,
    Protected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEdge {
    pub node_id: u64,
    pub relation: EvidenceRelation,
    pub subject: String,
    pub source: String,
    pub observed_at_unix_ms: u64,
    /// Evidence quality, from 0 to 100; never a deletion authorization.
    pub confidence: u8,
}

/// One immutable snapshot and its separately collected evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiskGraph {
    pub snapshot: DiskSnapshot,
    pub nodes: Vec<DiskNode>,
    pub evidence: Vec<EvidenceEdge>,
}

impl DiskGraph {
    /// The node the walk started from: the one with no parent.
    ///
    /// Panics when a graph has no root, which `validate_graph` rules out at
    /// every write path, so this is a programming error rather than a state a
    /// stored snapshot can be in.
    pub fn root(&self) -> &DiskNode {
        self.nodes
            .iter()
            .find(|node| node.parent_id.is_none())
            .expect("a validated graph has a root")
    }
}
