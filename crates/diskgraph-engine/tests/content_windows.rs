//! Windows 原生内容验收：仅使用临时 NTFS 文件，不使用真实用户目录或云账户。
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use diskgraph_core::{
    Authorizer, BusinessError, Decision, Grant, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};
use diskgraph_engine::content::{
    ConservativeProbe, InspectionRequest, InspectionStop, PlaceholderProbe,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use sha2::{Digest, Sha256};

struct NativeContent {
    engine: Arc<Engine>,
    _workspace: tempfile::TempDir,
    root: PathBuf,
    scope: ScopeId,
    principal: PrincipalId,
    policy: PolicyAuthorizer,
}

impl NativeContent {
    fn open() -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let engine = Arc::new(
            Engine::open(EngineConfig {
                data_dir: workspace.path().join("data"),
                ..EngineConfig::default()
            })
            .unwrap(),
        );
        let principal = PrincipalId::new("windows-content-fixture").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let policy_version = engine.control_store().unwrap().policy_version().unwrap();
        engine
            .control_store()
            .unwrap()
            .upsert_grant(&Grant {
                principal: principal.clone(),
                permission: Permission::ContentRead,
                scope: scope.clone(),
                policy_version,
            })
            .unwrap();
        let policy = engine.policy_authorizer().unwrap();
        Self {
            engine,
            _workspace: workspace,
            root,
            scope,
            principal,
            policy,
        }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn request<'a>(&'a self, path: &'a Path) -> InspectionRequest<'a> {
        InspectionRequest {
            scope_id: &self.scope,
            principal: &self.principal,
            path,
            offset: 0,
            max_bytes: 1 << 20,
            cancel: None,
            chunk_bytes: 3,
        }
    }
}

#[test]
fn native_windows_content_reads_ranges_and_confirms_complete_digests() {
    let fixture = NativeContent::open();
    let bytes = "你好/Windows content/🙂".as_bytes();
    let path = fixture.file("内容🙂.txt", bytes);
    let request = InspectionRequest {
        offset: 6,
        max_bytes: 8,
        ..fixture.request(&path)
    };
    let read = fixture
        .engine
        .read_bounded(&request, &ConservativeProbe, &fixture.policy)
        .unwrap();
    assert_eq!(read.bytes, bytes[6..14]);
    assert!(read.truncated);
    assert_eq!(read.stopped, None);
    let digest = fixture
        .engine
        .digest_bounded(&fixture.request(&path), &ConservativeProbe, &fixture.policy)
        .unwrap();
    assert!(digest.confirmed());
    assert_eq!(digest.bytes_digested, bytes.len() as u64);
    assert_eq!(digest.digest_hex, hex::encode(Sha256::digest(bytes)));
}

#[test]
fn native_windows_digest_cannot_confirm_an_over_budget_file() {
    let fixture = NativeContent::open();
    let path = fixture.file("large.bin", &[7; 1024]);
    let request = InspectionRequest {
        max_bytes: 1,
        chunk_bytes: usize::MAX,
        ..fixture.request(&path)
    };
    let digest = fixture
        .engine
        .digest_bounded(&request, &ConservativeProbe, &fixture.policy)
        .unwrap();
    assert_eq!(digest.bytes_digested, 1);
    assert_eq!(digest.stopped, Some(InspectionStop::ByteLimit));
    assert!(!digest.confirmed());
    assert!(digest.digest_hex.is_empty());
}

#[test]
fn native_windows_cancellation_and_deadline_never_confirm_content() {
    let fixture = NativeContent::open();
    let path = fixture.file("cancel.bin", b"keep this private");
    let cancel = AtomicBool::new(true);
    let request = InspectionRequest {
        cancel: Some(&cancel),
        ..fixture.request(&path)
    };
    let digest = fixture
        .engine
        .digest_bounded(&request, &ConservativeProbe, &fixture.policy)
        .unwrap();
    assert_eq!(digest.stopped, Some(InspectionStop::Cancelled));
    assert_eq!(digest.bytes_digested, 0);
    assert!(digest.digest_hex.is_empty());
    let digest = fixture
        .engine
        .digest_bounded_until(
            &fixture.request(&path),
            &ConservativeProbe,
            &fixture.policy,
            std::time::Instant::now(),
        )
        .unwrap();
    assert_eq!(digest.stopped, Some(InspectionStop::Deadline));
    assert_eq!(digest.bytes_digested, 0);
    assert!(digest.digest_hex.is_empty());
}

#[test]
fn native_windows_offline_attribute_is_skipped_even_with_a_false_probe() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_OFFLINE, SetFileAttributesW};
    struct FalseProbe;
    impl PlaceholderProbe for FalseProbe {
        fn is_placeholder(&self, _: &Path) -> bool {
            false
        }
    }
    let fixture = NativeContent::open();
    let path = fixture.file(
        "offline.bin",
        b"attribute fixture, not a real cloud provider",
    );
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    assert_ne!(
        unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_OFFLINE) },
        0
    );
    let read = fixture
        .engine
        .read_bounded(&fixture.request(&path), &FalseProbe, &fixture.policy)
        .unwrap();
    assert_eq!(read.stopped, Some(InspectionStop::Placeholder));
    assert!(read.bytes.is_empty());
    let digest = fixture
        .engine
        .digest_bounded(&fixture.request(&path), &FalseProbe, &fixture.policy)
        .unwrap();
    assert_eq!(digest.stopped, Some(InspectionStop::Placeholder));
    assert_eq!(digest.bytes_digested, 0);
    assert!(digest.digest_hex.is_empty());
}

#[test]
fn native_windows_content_requires_the_current_grant() {
    let fixture = NativeContent::open();
    let path = fixture.file("secret.bin", b"private");
    fixture
        .engine
        .control_store()
        .unwrap()
        .revoke_grant(&fixture.principal, &Permission::ContentRead, &fixture.scope)
        .unwrap();
    assert!(matches!(
        fixture
            .engine
            .read_bounded(&fixture.request(&path), &ConservativeProbe, &fixture.policy),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn native_windows_active_writer_conflicts_before_content_access() {
    let fixture = NativeContent::open();
    let path = fixture.file("held.bin", b"private");
    let _writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    assert!(matches!(
        fixture
            .engine
            .read_bounded(&fixture.request(&path), &ConservativeProbe, &fixture.policy),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
}

#[test]
fn native_windows_inspection_keeps_its_parent_held_until_finished() {
    struct ParentProbe {
        root: PathBuf,
        blocked: AtomicBool,
    }
    impl PlaceholderProbe for ParentProbe {
        fn is_placeholder(&self, _: &Path) -> bool {
            self.blocked.store(
                std::fs::rename(&self.root, self.root.with_extension("moved")).is_err(),
                Ordering::SeqCst,
            );
            false
        }
    }
    let fixture = NativeContent::open();
    let path = fixture.file("held.bin", b"stable");
    let probe = ParentProbe {
        root: fixture.root.clone(),
        blocked: AtomicBool::new(false),
    };
    let read = fixture
        .engine
        .read_bounded(&fixture.request(&path), &probe, &fixture.policy);
    assert!(probe.blocked.load(Ordering::SeqCst));
    assert_eq!(read.unwrap().bytes, b"stable");
    std::fs::rename(&fixture.root, fixture.root.with_extension("moved")).unwrap();
}

#[test]
fn native_windows_revoke_during_acquisition_does_not_return_body() {
    struct RevokeProbe {
        engine: Arc<Engine>,
        principal: PrincipalId,
        scope: ScopeId,
    }
    impl PlaceholderProbe for RevokeProbe {
        fn is_placeholder(&self, _: &Path) -> bool {
            self.engine
                .control_store()
                .unwrap()
                .revoke_grant(&self.principal, &Permission::ContentRead, &self.scope)
                .unwrap();
            false
        }
    }
    let fixture = NativeContent::open();
    let path = fixture.file("revoked.bin", b"private");
    let probe = RevokeProbe {
        engine: fixture.engine.clone(),
        principal: fixture.principal.clone(),
        scope: fixture.scope.clone(),
    };
    assert!(matches!(
        fixture
            .engine
            .read_bounded(&fixture.request(&path), &probe, &fixture.policy),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn native_windows_scope_escapes_and_alternate_streams_are_refused() {
    let fixture = NativeContent::open();
    let path = fixture.file("base.bin", b"base");
    let stream = fixture.root.join("base.bin:private");
    std::fs::write(&stream, b"hidden stream").unwrap();
    let outside = fixture._workspace.path().join("outside.bin");
    std::fs::write(&outside, b"outside").unwrap();
    for path in [
        &stream,
        &outside,
        &fixture.root.join("..\\outside.bin"),
        &PathBuf::from("\\\\.\\NUL"),
    ] {
        assert!(
            fixture
                .engine
                .read_bounded(&fixture.request(path), &ConservativeProbe, &fixture.policy)
                .is_err()
        );
    }
    assert_eq!(std::fs::read(path).unwrap(), b"base");
}

#[test]
fn native_windows_junction_cannot_redirect_a_content_request() {
    let fixture = NativeContent::open();
    let outside = fixture._workspace.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret.bin"), b"outside private").unwrap();
    let junction = fixture.root.join("junction");
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "junction fixture failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let path = junction.join("secret.bin");
    assert!(
        fixture
            .engine
            .read_bounded(&fixture.request(&path), &ConservativeProbe, &fixture.policy)
            .is_err()
    );
    assert_eq!(
        std::fs::read(outside.join("secret.bin")).unwrap(),
        b"outside private"
    );
    std::fs::remove_dir(&junction).unwrap();
}

#[test]
fn native_windows_acquisition_mutation_returns_conflict_before_read_or_digest() {
    struct WriterProbe {
        blocked: Mutex<Option<bool>>,
    }
    impl PlaceholderProbe for WriterProbe {
        fn is_placeholder(&self, path: &Path) -> bool {
            *self.blocked.lock().unwrap() = Some(
                std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(path)
                    .is_err(),
            );
            false
        }
    }
    let fixture = NativeContent::open();
    let path = fixture.file("stable.bin", b"stable bytes");
    let probe = WriterProbe {
        blocked: Mutex::new(None),
    };
    let result = fixture
        .engine
        .read_bounded(&fixture.request(&path), &probe, &fixture.policy);
    // 仅属性访问不冻结新 writer；必须在申请数据之前拒绝被改变的版本。
    assert_eq!(*probe.blocked.lock().unwrap(), Some(false));
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    assert!(std::fs::read(&path).unwrap().is_empty());
    std::fs::write(&path, b"stable bytes").unwrap();
    assert!(matches!(
        fixture
            .engine
            .digest_bounded(&fixture.request(&path), &probe, &fixture.policy),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
}

#[test]
fn native_windows_data_handle_blocks_new_writer_for_reads_and_digests() {
    struct ReadPhaseAuthorizer {
        policy: PolicyAuthorizer,
        path: PathBuf,
        calls: AtomicUsize,
        blocked: AtomicBool,
    }
    impl Authorizer for ReadPhaseAuthorizer {
        fn policy_version(&self) -> u64 {
            self.policy.policy_version()
        }
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            // 第二次授权在数据句柄获取后、第一个读取块之前；无需时间竞态或 sleep。
            if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
                self.blocked.store(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .truncate(true)
                        .open(&self.path)
                        .is_err(),
                    Ordering::SeqCst,
                );
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let fixture = NativeContent::open();
    let path = fixture.file("read-phase.bin", b"stable bytes");
    for digest in [false, true] {
        let authorizer = ReadPhaseAuthorizer {
            policy: fixture.policy.clone(),
            path: path.clone(),
            calls: AtomicUsize::new(0),
            blocked: AtomicBool::new(false),
        };
        if digest {
            let result = fixture
                .engine
                .digest_bounded(&fixture.request(&path), &ConservativeProbe, &authorizer)
                .unwrap();
            assert!(result.confirmed());
            assert_eq!(
                result.digest_hex,
                hex::encode(Sha256::digest(b"stable bytes"))
            );
        } else {
            let result = fixture
                .engine
                .read_bounded(&fixture.request(&path), &ConservativeProbe, &authorizer)
                .unwrap();
            assert_eq!(result.stopped, None);
            assert_eq!(result.bytes, b"stable bytes");
        }
        assert!(authorizer.calls.load(Ordering::SeqCst) >= 2);
        assert!(authorizer.blocked.load(Ordering::SeqCst));
        assert_eq!(std::fs::read(&path).unwrap(), b"stable bytes");
    }
}
