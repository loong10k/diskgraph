//! 真实 CFAPI 非物化内容合同；隔离临时同步根，不读取个人云目录。
#![cfg(windows)]
#[path = "cloud_fixture/windows_cloud_provider.rs"]
mod windows_cloud_provider;

use diskgraph_core::{BusinessError, Grant, Permission, PrincipalId};
use diskgraph_engine::content::{ConservativeProbe, InspectionRequest, InspectionStop};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use std::io::Read;
use windows_cloud_provider::WindowsCloudProvider;

#[test]
fn actual_cloud_placeholder_content_never_fetches_and_unguarded_control_does() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("isolated-sync-root");
    std::fs::create_dir(&root).unwrap();
    let mut provider = WindowsCloudProvider::connect(&root)
        .expect("actual CFAPI provider must connect; unavailable is not acceptance");
    let path = provider.create_placeholder().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: workspace.path().join("isolated-control"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("isolated-cloud-content").unwrap();
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
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 8192,
        cancel: None,
        chunk_bytes: 4096,
    };
    // 只允许明确占位结果或严格原生 Unsupported；任意 I/O 失败不能冒充非物化证明。
    match engine.read_bounded(&request, &ConservativeProbe, &policy) {
        Ok(read) => {
            assert!(read.bytes.is_empty());
            assert_eq!(read.stopped, Some(InspectionStop::Placeholder));
        }
        Err(EngineError::Business(BusinessError::Unsupported)) => {}
        other => panic!("actual cloud read must refuse content without recall: {other:?}"),
    }
    match engine.digest_bounded(&request, &ConservativeProbe, &policy) {
        Ok(digest) => {
            assert_eq!(digest.bytes_digested, 0);
            assert_eq!(digest.stopped, Some(InspectionStop::Placeholder));
            assert!(digest.digest_hex.is_empty() && !digest.confirmed());
        }
        Err(EngineError::Business(BusinessError::Unsupported)) => {}
        other => panic!("actual cloud digest must remain unconfirmed without recall: {other:?}"),
    }
    assert_eq!(
        provider.fetches(),
        0,
        "product content path triggered real FETCH_DATA"
    );
    // 正控必须在产品观察之后执行：真实未保护读取需要提供方，提供方明确拒绝且从不供给正文。
    let mut byte = [0_u8; 1];
    let unsafe_read = std::fs::File::open(&path).and_then(|mut file| file.read_exact(&mut byte));
    assert!(
        unsafe_read.is_err(),
        "test provider unexpectedly supplied cloud content"
    );
    assert!(
        provider.fetches() > 0,
        "ordinary read did not exercise actual FETCH_DATA wiring"
    );
    assert_eq!(
        provider.callback_failure(),
        0,
        "CFAPI rejection completion failed"
    );
    provider.finish().unwrap();
    println!("DG_REAL_CFAPI_PRODUCT_NO_FETCH_AND_ORDINARY_FETCH_CONTROL=1");
}
