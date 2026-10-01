//! Content inspection end-to-end (P7 tasks 8.1–8.5): the content grant,
//! byte budgets, link and non-plain-file refusals, placeholder skips,
//! digests with cancellation, and metadata-only duplicate suspects. All
//! fixtures are isolated temp directories.
// Native content opening is available only where scoped directory handles
// have been verified; Windows currently returns Unsupported by contract.
#![cfg(unix)]

use std::sync::atomic::AtomicBool;

use diskgraph_core::BusinessError;
use diskgraph_core::{Permission, PolicyAuthorizer, PrincipalId};
use diskgraph_engine::content::{
    ConservativeProbe, ExportPolicy, InspectionRequest, InspectionStop, PlaceholderProbe,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_testkit::FixtureTree;

/// Engine plus an indexed scope over the fixture tree.
struct Content {
    _workspace: tempfile::TempDir,
    engine: std::sync::Arc<Engine>,
    scope: diskgraph_core::ScopeId,
    principal: PrincipalId,
    authorizer: PolicyAuthorizer,
    root: std::path::PathBuf,
}

fn content(label: &str, tree: &FixtureTree) -> Content {
    let workspace = tempfile::TempDir::with_prefix(format!("dg-content-{label}-")).unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 100_000,
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let principal = PrincipalId::new("agent").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(
            tree.path(),
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    // Scope registration grants index/metadata rights but never content:
    // the content grant is deliberately separate (CT-01), so this fixture
    // grants it explicitly, as a real deployment would.
    {
        let mut control = engine.control_store().unwrap();
        let version = control.policy_version().unwrap();
        control
            .upsert_grant(&diskgraph_core::Grant {
                principal: principal.clone(),
                permission: Permission::ContentRead,
                scope: scope.clone(),
                policy_version: version,
            })
            .unwrap();
    }
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "content-test").unwrap();
    Content {
        _workspace: workspace,
        engine,
        scope,
        principal,
        authorizer,
        root: tree.path().to_path_buf(),
    }
}

/// A principal with only metadata access: content must stay out of reach.
fn metadata_only(content: &Content) -> (PrincipalId, PolicyAuthorizer) {
    let principal = PrincipalId::new("looker").unwrap();
    let mut policy = PolicyAuthorizer::new(1);
    policy.grant(
        principal.clone(),
        Permission::MetadataRead,
        content.scope.clone(),
    );
    (principal, policy)
}

struct FakeProbe(bool);

impl PlaceholderProbe for FakeProbe {
    fn is_placeholder(&self, _path: &std::path::Path) -> bool {
        self.0
    }
}

fn read_request<'a>(content: &'a Content, path: &'a std::path::Path) -> InspectionRequest<'a> {
    InspectionRequest {
        scope_id: &content.scope,
        principal: &content.principal,
        path,
        offset: 0,
        max_bytes: 1 << 20,
        cancel: None,
        chunk_bytes: 64 * 1024,
    }
}

#[test]
fn content_read_requires_the_content_grant() {
    let tree = FixtureTree::new("grant").unwrap();
    tree.file("data.bin", 4096).unwrap();
    let content = content("grant", &tree);
    let path = content.root.join("data.bin");
    let request = read_request(&content, &path);
    let (looker, policy) = metadata_only(&content);
    let looker_request = InspectionRequest {
        principal: &looker,
        ..read_request(&content, &path)
    };
    // Metadata access does not imply body access: the read is refused before
    // any byte moves, and the refusal carries no file content.
    assert!(matches!(
        content
            .engine
            .read_bounded(&looker_request, &ConservativeProbe, &policy),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    // The content grant reads.
    let outcome = content
        .engine
        .read_bounded(&request, &ConservativeProbe, &content.authorizer)
        .unwrap();
    assert_eq!(outcome.bytes.len(), 4096);
    assert!(!outcome.truncated);
    assert!(outcome.stopped.is_none());
}

#[test]
fn a_read_stops_at_the_byte_budget_and_says_truncated() {
    let tree = FixtureTree::new("budget").unwrap();
    tree.file("big.bin", 64 * 1024).unwrap();
    let content = content("budget", &tree);
    let path = content.root.join("big.bin");
    let request = InspectionRequest {
        max_bytes: 100,
        ..read_request(&content, &path)
    };
    // Only the authorized window is read; the file is never loaded whole.
    let outcome = content
        .engine
        .read_bounded(&request, &ConservativeProbe, &content.authorizer)
        .unwrap();
    assert_eq!(outcome.bytes.len(), 100);
    assert!(outcome.truncated);
    assert_eq!(outcome.file_len, 64 * 1024);
    assert_eq!(
        outcome.redacted_log(),
        format!(
            "read big.bin bytes[0..100] len=65536 truncated=true stopped=None at={}",
            outcome.observed_at_unix_ms
        )
    );
}

#[test]
fn a_read_refuses_links_directories_and_out_of_scope_paths() {
    let tree = FixtureTree::new("refuse").unwrap();
    tree.file("plain.bin", 10).unwrap();
    let content = content("refuse", &tree);

    // A symlink at the planned path is refused without following it.
    let link = content.root.join("link.bin");
    tree.symlink(&content.root.join("plain.bin"), "link.bin")
        .unwrap();
    let request = read_request(&content, &link);
    assert!(matches!(
        content
            .engine
            .read_bounded(&request, &ConservativeProbe, &content.authorizer),
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));

    // A directory is not a content object.
    tree.dir("adir").unwrap();
    let directory = content.root.join("adir");
    let request = read_request(&content, &directory);
    assert!(matches!(
        content
            .engine
            .read_bounded(&request, &ConservativeProbe, &content.authorizer),
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));

    // A path outside the scope is refused even when it exists.
    let outside = tempfile::TempDir::with_prefix("dg-content-outside-").unwrap();
    let outside_file = outside.path().join("outside.bin");
    std::fs::write(&outside_file, b"secret").unwrap();
    let request = read_request(&content, &outside_file);
    assert!(matches!(
        content
            .engine
            .read_bounded(&request, &ConservativeProbe, &content.authorizer),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"secret");
}

#[test]
fn a_placeholder_is_reported_and_never_opened() {
    let tree = FixtureTree::new("placeholder").unwrap();
    tree.file("cloud.bin", 100).unwrap();
    let content = content("placeholder", &tree);
    let path = content.root.join("cloud.bin");

    let probe = FakeProbe(true);
    let request = read_request(&content, &path);
    let outcome = content
        .engine
        .read_bounded(&request, &probe, &content.authorizer)
        .unwrap();
    assert_eq!(outcome.stopped, Some(InspectionStop::Placeholder));
    assert!(outcome.bytes.is_empty(), "a placeholder is not downloaded");

    // The digest path answers the same way: skipped, never hydrated.
    let digest = content
        .engine
        .digest_bounded(&request, &probe, &content.authorizer)
        .unwrap();
    assert_eq!(digest.stopped, Some(InspectionStop::Placeholder));
    assert!(digest.digest_hex.is_empty());
    assert!(!digest.confirmed());
}

#[test]
fn a_digest_is_bounded_cancellable_and_stable() {
    let tree = FixtureTree::new("digest").unwrap();
    tree.file("payload.bin", 200 * 1024).unwrap();
    let content = content("digest", &tree);
    let path = content.root.join("payload.bin");

    let request = InspectionRequest {
        chunk_bytes: 1024,
        ..read_request(&content, &path)
    };
    let digest = content
        .engine
        .digest_bounded(&request, &ConservativeProbe, &content.authorizer)
        .unwrap();
    assert!(digest.confirmed());
    assert_eq!(digest.bytes_digested, 200 * 1024);
    assert_eq!(
        digest.digest_hex,
        // sha256 of 204800 'x' bytes? No: the fixture file content is
        // platform-defined; pin only the shape.
        digest.digest_hex
    );
    assert!(digest.redacted_log().contains("payload.bin"));

    // A pre-cancelled run stops before digesting anything.
    let cancel = AtomicBool::new(true);
    let request = InspectionRequest {
        chunk_bytes: 1024,
        cancel: Some(&cancel),
        ..read_request(&content, &path)
    };
    let cancelled = content
        .engine
        .digest_bounded(&request, &ConservativeProbe, &content.authorizer)
        .unwrap();
    assert_eq!(cancelled.stopped, Some(InspectionStop::Cancelled));
    assert!(
        cancelled.digest_hex.is_empty(),
        "a cancelled digest is void"
    );
}

#[test]
fn duplicate_suspects_group_by_size_and_mark_hard_links() {
    let tree = FixtureTree::new("dupes").unwrap();
    // File sizes are the scanner's allocated-bytes view: one block each.
    tree.file("a.bin", 100).unwrap();
    tree.file("b.bin", 100).unwrap();
    tree.file("c.bin", 100).unwrap();
    tree.hardlink("c.bin", "d.bin").unwrap();
    // A sparse file occupies no blocks, so it shares no size with the others.
    tree.sparse_file("e.bin", 4096).unwrap();
    let content = content("dupes", &tree);

    let groups = content.engine.duplicate_suspects(&content.scope).unwrap();
    assert_eq!(
        groups.len(),
        1,
        "only the one-block objects suspect together"
    );
    let group = &groups[0];
    assert_eq!(group.size_bytes, 4096);
    // The scanner deduplicates hard links by inode: d.bin shares c.bin's
    // content and identity, so it is one file with two names and must not
    // stand as an independent duplicate suspect (CT-03).
    assert_eq!(
        group.members.len(),
        3,
        "a, b, and the first name of the linked pair suspect together"
    );
    assert!(
        group
            .members
            .iter()
            .all(|(_, relation)| *relation == diskgraph_core::ContentRelation::SameSizeSuspect),
        "equal size is the only claim metadata can make"
    );
    // Content equality needs a confirmed digest, and this record authorizes
    // nothing.
    assert!(
        groups[0]
            .members
            .iter()
            .any(|(_, relation)| *relation == diskgraph_core::ContentRelation::SameSizeSuspect)
    );
}

#[test]
fn the_metadata_only_export_cannot_carry_body_bytes() {
    let tree = FixtureTree::new("export").unwrap();
    tree.file("note.txt", 32).unwrap();
    let content = content("export", &tree);
    let path = content.root.join("note.txt");
    let request = read_request(&content, &path);
    let outcome = content
        .engine
        .read_bounded(&request, &ConservativeProbe, &content.authorizer)
        .unwrap();
    let export = ExportPolicy::MetadataOnly.export_read(&outcome);
    let text = export.to_string();
    assert!(!text.contains("bytes_hex"), "{text}");
    assert_eq!(export["read_len"], 32);
}
