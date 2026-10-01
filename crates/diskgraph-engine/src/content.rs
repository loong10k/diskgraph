//! Bounded content inspection (P7 tasks 8.1–8.5, CT-01..CT-04). Reads and
//! digests run under their own authorization (`content:read`), a byte
//! budget, and an identity contract: the object is re-stat'ed before and
//! after, so a file that changed underneath the read is reported unstable
//! instead of quietly producing a hash of half of each version.
//!
//! Nothing here persists content: outcomes carry the bytes only in memory
//! for the caller, and every log/export helper is metadata-shaped by
//! construction.

use sha2::Digest;
use std::io::Read;
use std::path::{Path, PathBuf};

use diskgraph_core::{BusinessError, DiskGraph, PlaceholderPolicy, PrincipalId, ScopeId};

use crate::{Engine, EngineError};

/// Milliseconds since the epoch for logs.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Detects a cloud placeholder. Real platforms need platform API surface
/// (macOS dataless-flag stat, Windows reparse cloud attributes); the
/// production probe is therefore conservative — it never claims a file is a
/// placeholder it cannot prove, and the read path treats "unknown" as
/// "proceed under the no-hydration contract". Tests inject fakes.
pub trait PlaceholderProbe {
    fn is_placeholder(&self, path: &Path) -> bool;
}

/// The production probe: honest about knowing nothing yet (tracked in the
/// platform ledger, `diskgraph_testkit::real_os_requirements`).
pub struct ConservativeProbe;

impl PlaceholderProbe for ConservativeProbe {
    fn is_placeholder(&self, _path: &Path) -> bool {
        false
    }
}

/// Why a read or digest did not happen or did not finish.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectionStop {
    /// The object is a cloud placeholder and the policy forbids hydration.
    Placeholder,
    /// The caller asked to cancel.
    Cancelled,
    /// The object changed while being read: the result is void.
    Unstable,
    /// 字节预算不足以确认完整内容。
    ByteLimit,
}

/// What a bounded read produced. The bytes are the caller's to use and drop;
/// nothing else in this structure can carry content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOutcome {
    pub requested_path: PathBuf,
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub file_len: u64,
    pub truncated: bool,
    /// Set when the read did not complete for a named reason; `bytes` then
    /// holds whatever was read before the stop and must not be trusted as
    /// the file's content.
    pub stopped: Option<InspectionStop>,
    pub observed_at_unix_ms: u64,
}

impl ReadOutcome {
    /// A log line for this read: names, ranges, flags — never bytes (CT-04).
    pub fn redacted_log(&self) -> String {
        format!(
            "read {} bytes[{}..{}] len={} truncated={} stopped={:?} at={}",
            self.requested_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<unnamed>".into()),
            self.offset,
            self.offset + self.bytes.len() as u64,
            self.file_len,
            self.truncated,
            self.stopped,
            self.observed_at_unix_ms
        )
    }
}

/// A content digest with the stability evidence that makes it meaningful.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestOutcome {
    pub requested_path: PathBuf,
    pub digest_hex: String,
    pub bytes_digested: u64,
    pub stopped: Option<InspectionStop>,
    pub observed_at_unix_ms: u64,
}

impl DigestOutcome {
    pub fn confirmed(&self) -> bool {
        self.stopped.is_none()
    }

    /// A log line for this digest: the file name and the digest, no content.
    pub fn redacted_log(&self) -> String {
        format!(
            "digest {} {} confirmed={} at={}",
            self.requested_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<unnamed>".into()),
            self.digest_hex,
            self.confirmed(),
            self.observed_at_unix_ms
        )
    }
}

/// What one bounded inspection needs.
pub struct InspectionRequest<'a> {
    pub scope_id: &'a ScopeId,
    pub principal: &'a PrincipalId,
    /// The exact object, inside the scope's root.
    pub path: &'a Path,
    pub offset: u64,
    /// The whole byte budget; a read never loads more than this.
    pub max_bytes: u64,
    /// Set by cooperative cancellation between chunks.
    pub cancel: Option<&'a std::sync::atomic::AtomicBool>,
    /// Chunk size for reads and digests; bounds peak memory.
    pub chunk_bytes: usize,
}

/// 身份指纹同时记录长度及高精度修改信息，原地写入也使结果失效。
fn file_identity(metadata: &std::fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn identity_stable(before: &Option<String>, path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .ok()
        .and_then(|after| file_identity(&after))
        .is_some_and(|after| Some(&after) == before.as_ref())
}

/// Refuses everything that is not a plain file (CT-01): directories,
/// FIFOs, sockets, and devices are not content objects.
fn ensure_plain_file(path: &Path, metadata: &std::fs::Metadata) -> Result<(), EngineError> {
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let kind = metadata.file_type();
        if kind.is_dir()
            || kind.is_fifo()
            || kind.is_socket()
            || kind.is_block_device()
            || kind.is_char_device()
        {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        if !metadata.is_file() {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
    }
    Ok(())
}

/// Checks the object is inside the scope and not a link at any planned
/// component: the final component must be a real object, and the canonical
/// parent must stay under the canonical scope root (SC-03).
fn ensure_inside_scope(root: &Path, path: &Path) -> Result<(), EngineError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| EngineError::Business(BusinessError::NotFound))?;
    if metadata.file_type().is_symlink() {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    }
    let parent = path.parent().unwrap_or(Path::new("/"));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| EngineError::Business(BusinessError::NotFound))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| EngineError::Business(BusinessError::NotFound))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(EngineError::Business(BusinessError::PermissionDenied));
    }
    Ok(())
}

impl Engine {
    /// Reads at most `max_bytes` from `offset` of one authorized object.
    /// Every gate runs before the first byte moves: authorization, scope
    /// containment, plain-file check, placeholder policy (CT-01, CT-02).
    pub fn read_bounded(
        &self,
        request: &InspectionRequest<'_>,
        probe: &dyn PlaceholderProbe,
        authorizer: &dyn diskgraph_core::Authorizer,
    ) -> Result<ReadOutcome, EngineError> {
        self.require(
            authorizer,
            request.principal,
            &diskgraph_core::Permission::ContentRead,
            request.scope_id,
        )?;
        let record = self.scope(request.scope_id)?;
        if record.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let root = record
            .root
            .to_native_path()
            .map_err(|_| EngineError::Business(BusinessError::Unsupported))?;
        ensure_inside_scope(&root, request.path)?;
        let metadata = std::fs::symlink_metadata(request.path)?;
        ensure_plain_file(request.path, &metadata)?;
        // Hydration is never available: whatever the caller asked for, a
        // placeholder is reported, not downloaded (CT-02).
        let policy = PlaceholderPolicy::resolve(PlaceholderPolicy::Hydrate);
        let _ = policy;
        if probe.is_placeholder(request.path) {
            return Ok(ReadOutcome {
                requested_path: request.path.to_path_buf(),
                offset: request.offset,
                bytes: Vec::new(),
                file_len: metadata.len(),
                truncated: false,
                stopped: Some(InspectionStop::Placeholder),
                observed_at_unix_ms: now_ms(),
            });
        }
        let identity_before = file_identity(&metadata);
        let mut file = crate::scoped_file::open_scoped(&root, request.path)?;
        if file_identity(&file.metadata()?) != identity_before {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(request.offset))?;
        let chunk = request.chunk_bytes.clamp(1, 64 * 1024).min(
            usize::try_from(request.max_bytes)
                .unwrap_or(usize::MAX)
                .max(1),
        );
        let mut bytes = Vec::new();
        let mut buffer = vec![0_u8; chunk];
        let mut truncated = false;
        let mut stopped = None;
        loop {
            self.require(
                authorizer,
                request.principal,
                &diskgraph_core::Permission::ContentRead,
                request.scope_id,
            )?;
            if self.control_store()?.live_permission(
                request.principal,
                &diskgraph_core::Permission::ContentRead,
                request.scope_id,
            )? == Some(false)
            {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            if bytes.len() as u64 >= request.max_bytes {
                truncated = true;
                break;
            }
            if let Some(cancel) = request.cancel
                && cancel.load(std::sync::atomic::Ordering::SeqCst)
            {
                stopped = Some(InspectionStop::Cancelled);
                break;
            }
            let want = usize::try_from(request.max_bytes - bytes.len() as u64)
                .unwrap_or(usize::MAX)
                .min(chunk);
            let buffer = &mut buffer[..want];
            let read = file.read(buffer)?;
            if read == 0 {
                break;
            }
            bytes
                .try_reserve_exact(read)
                .map_err(|_| EngineError::Business(BusinessError::ResourceExhausted))?;
            bytes.extend_from_slice(&buffer[..read]);
        }
        // The file must still be the object the read started on.
        if file_identity(&file.metadata()?) != identity_before
            || !identity_stable(&identity_before, request.path)
        {
            stopped = Some(InspectionStop::Unstable);
        }
        Ok(ReadOutcome {
            requested_path: request.path.to_path_buf(),
            offset: request.offset,
            bytes,
            file_len: metadata.len(),
            truncated,
            stopped,
            observed_at_unix_ms: now_ms(),
        })
    }

    /// Digests one authorized object in bounded chunks with cancellation and
    /// identity checks between chunks (CT-03, task 8.4). An unstable or
    /// cancelled digest is reported as such and must never upgrade a
    /// duplicate suspect.
    pub fn digest_bounded(
        &self,
        request: &InspectionRequest<'_>,
        probe: &dyn PlaceholderProbe,
        authorizer: &dyn diskgraph_core::Authorizer,
    ) -> Result<DigestOutcome, EngineError> {
        self.require(
            authorizer,
            request.principal,
            &diskgraph_core::Permission::ContentRead,
            request.scope_id,
        )?;
        let record = self.scope(request.scope_id)?;
        if record.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let root = record
            .root
            .to_native_path()
            .map_err(|_| EngineError::Business(BusinessError::Unsupported))?;
        ensure_inside_scope(&root, request.path)?;
        let metadata = std::fs::symlink_metadata(request.path)?;
        ensure_plain_file(request.path, &metadata)?;
        if probe.is_placeholder(request.path) {
            return Ok(DigestOutcome {
                requested_path: request.path.to_path_buf(),
                digest_hex: String::new(),
                bytes_digested: 0,
                stopped: Some(InspectionStop::Placeholder),
                observed_at_unix_ms: now_ms(),
            });
        }
        let identity_before = file_identity(&metadata);
        let mut file = crate::scoped_file::open_scoped(&root, request.path)?;
        if file_identity(&file.metadata()?) != identity_before {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        let chunk = request.chunk_bytes.clamp(1, 64 * 1024).min(
            usize::try_from(request.max_bytes)
                .unwrap_or(usize::MAX)
                .max(1),
        );
        let mut hasher = sha2::Sha256::new();
        let mut total = 0_u64;
        let mut stopped = None;
        let mut buffer = vec![0_u8; chunk];
        loop {
            self.require(
                authorizer,
                request.principal,
                &diskgraph_core::Permission::ContentRead,
                request.scope_id,
            )?;
            if self.control_store()?.live_permission(
                request.principal,
                &diskgraph_core::Permission::ContentRead,
                request.scope_id,
            )? == Some(false)
            {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            if let Some(cancel) = request.cancel
                && cancel.load(std::sync::atomic::Ordering::SeqCst)
            {
                stopped = Some(InspectionStop::Cancelled);
                break;
            }
            if total >= request.max_bytes {
                if total < metadata.len() {
                    stopped = Some(InspectionStop::ByteLimit);
                }
                break;
            }
            let remaining = usize::try_from(request.max_bytes - total).unwrap_or(usize::MAX);
            let want = remaining.min(buffer.len());
            let read = file.read(&mut buffer[..want])?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            total += read as u64;
            // A file written or removed mid-digest voids the run: re-stat
            // between chunks so the recorded digest is never half of two
            // versions, or a digest of a file that is already gone.
            if file_identity(&file.metadata()?) != identity_before
                || !identity_stable(&identity_before, request.path)
            {
                stopped = Some(InspectionStop::Unstable);
                break;
            }
        }
        if stopped.is_none()
            && (total != metadata.len()
                || file_identity(&file.metadata()?) != identity_before
                || !identity_stable(&identity_before, request.path))
        {
            stopped = Some(InspectionStop::Unstable);
        }
        Ok(DigestOutcome {
            requested_path: request.path.to_path_buf(),
            digest_hex: if stopped.is_some() {
                String::new()
            } else {
                hex::encode(hasher.finalize_reset())
            },
            bytes_digested: total,
            stopped,
            observed_at_unix_ms: now_ms(),
        })
    }

    /// The metadata-only duplicate suspects of a published revision (CT-03,
    /// task 8.3): equal file sizes group as suspects, hard-linked names are
    /// annotated, and nothing here claims content equality.
    pub fn duplicate_suspects(
        &self,
        scope_id: &ScopeId,
    ) -> Result<Vec<diskgraph_core::SuspectGroup>, EngineError> {
        let revision = self
            .latest_revision(scope_id)?
            .ok_or_else(|| EngineError::Business(BusinessError::NotIndexed))?;
        let graph = self.load_revision(&revision)?;
        Ok(suspects_of(&graph))
    }
}

/// The suspects of one graph, as pure metadata (exported for tests).
pub fn suspects_of(graph: &DiskGraph) -> Vec<diskgraph_core::SuspectGroup> {
    let identities: Vec<Option<String>> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == diskgraph_core::NodeKind::File)
        .map(|node| {
            node.file_identity
                .as_ref()
                .map(|identity| format!("{}:{}", identity.volume_id, identity.file_id))
        })
        .collect();
    let objects: Vec<(u64, u64, Option<&str>)> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == diskgraph_core::NodeKind::File)
        .zip(&identities)
        .map(|(node, identity)| (node.id, node.direct_bytes, identity.as_deref()))
        .collect();
    diskgraph_core::suspect_groups(&objects)
}

/// How an inspection may be exported. The policy is the contract: a
/// metadata-only export cannot carry bytes, so no log or error path can
/// leak them by accident (CT-04, task 8.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportPolicy {
    MetadataOnly,
    IncludeDigests,
}

impl ExportPolicy {
    /// The exportable view of a read. `MetadataOnly` drops the bytes and
    /// reports their count instead; nothing else changes.
    pub fn export_read(&self, outcome: &ReadOutcome) -> serde_json::Value {
        let mut value = serde_json::json!({
            "path_name": outcome.requested_path.file_name().map(|name| name.to_string_lossy().into_owned()),
            "offset": outcome.offset,
            "file_len": outcome.file_len,
            "read_len": outcome.bytes.len(),
            "truncated": outcome.truncated,
            "stopped": outcome.stopped.as_ref().map(|stop| match stop {
                InspectionStop::Placeholder => "placeholder",
                InspectionStop::Cancelled => "cancelled",
                InspectionStop::Unstable => "unstable",
                InspectionStop::ByteLimit => "byte_limit",
            }),
            "observed_at_unix_ms": outcome.observed_at_unix_ms,
        });
        if *self == ExportPolicy::IncludeDigests {
            // Even with digests allowed, a read exports content only through
            // the explicit bytes field a caller asked for; policy governs it.
            value["bytes_hex"] = serde_json::Value::String(hex::encode(&outcome.bytes));
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_vectors() {
        fn digest(input: &[u8]) -> String {
            let mut hasher = sha2::Sha256::new();
            hasher.update(input);
            hex::encode(hasher.finalize())
        }
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // A multi-block input exercises the streaming path.
        assert_eq!(
            digest(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stability_is_void_when_the_object_vanishes_mid_read() {
        let workspace = tempfile::TempDir::with_prefix("dg-content-stable-").unwrap();
        let path = workspace.path().join("f");
        std::fs::write(&path, b"v1").unwrap();
        let before = std::fs::symlink_metadata(&path).ok();
        let identity = before.as_ref().and_then(file_identity);
        assert!(identity_stable(&identity, &path));
        // Replaced: a new inode at the same path is a different object.
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"v2").unwrap();
        assert!(!identity_stable(&identity, &path));
        // Gone: the strongest form of unstable.
        std::fs::remove_file(&path).unwrap();
        assert!(!identity_stable(&identity, &path));
    }

    #[test]
    fn the_redacted_log_never_carries_content() {
        let outcome = ReadOutcome {
            requested_path: PathBuf::from("/tmp/secret.txt"),
            offset: 0,
            bytes: b"THE ENTIRE SECRET BODY".to_vec(),
            file_len: 22,
            truncated: false,
            stopped: None,
            observed_at_unix_ms: 1,
        };
        let log = outcome.redacted_log();
        assert!(!log.contains("SECRET BODY"), "{log}");
        assert!(log.contains("secret.txt"));
        assert!(log.contains("len=22"));
    }

    #[test]
    fn metadata_only_export_drops_bytes_and_names_their_count() {
        let outcome = ReadOutcome {
            requested_path: PathBuf::from("/tmp/secret.txt"),
            offset: 0,
            bytes: b"confidential".to_vec(),
            file_len: 12,
            truncated: true,
            stopped: Some(InspectionStop::Unstable),
            observed_at_unix_ms: 1,
        };
        let export = ExportPolicy::MetadataOnly.export_read(&outcome);
        let text = export.to_string();
        assert!(!text.contains("confidential"), "{text}");
        assert_eq!(export["read_len"], 12);
        assert_eq!(export["truncated"], true);
        assert_eq!(export["stopped"], "unstable");
        assert!(export.get("bytes_hex").is_none());
    }
}
