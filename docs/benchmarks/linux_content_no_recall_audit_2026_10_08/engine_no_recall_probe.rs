//! 隔离真实 FUSE 提供方的产品不召回回归；缺少挂载环境是失败，不冒充验收。
use diskgraph_core::{Grant, Permission, PrincipalId, BusinessError};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_engine::content::{ConservativeProbe, InspectionRequest, InspectionStop};

#[test]
fn actual_provider_content_read_must_not_fetch() { check(false, false); }
#[test]
fn actual_provider_digest_must_not_fetch() { check(true, false); }
#[test]
fn nested_provider_content_read_must_not_fetch() { check(false, true); }
#[test]
fn nested_provider_digest_must_not_fetch() { check(true, true); }
fn check(digest: bool, nested: bool) {
 let root=std::path::PathBuf::from(std::env::var_os("DG_FUSE_ROOT").expect("isolated provider root"));
 let log=std::path::PathBuf::from(std::env::var_os("DG_FUSE_LOG").expect("original provider log"));
 let dir=tempfile::tempdir().unwrap();
 let engine=Engine::open(EngineConfig {data_dir:dir.path().join("data"),..EngineConfig::default()}).unwrap();
 let principal=PrincipalId::new("isolated-fuse-reader").unwrap();
 engine.bootstrap_local_admin(&principal).unwrap();
 let scope_root=if nested { root.parent().unwrap() } else { &root };
 let scope=engine.register_scope(scope_root,&principal,&engine.policy_authorizer().unwrap()).unwrap();
 let version=engine.control_store().unwrap().policy_version().unwrap();
 engine.control_store().unwrap().upsert_grant(&Grant{principal:principal.clone(),permission:Permission::ContentRead,scope:scope.clone(),policy_version:version}).unwrap();
 let policy=engine.policy_authorizer().unwrap();
 let path=root.join("placeholder");
 let request=InspectionRequest{scope_id:&scope,principal:&principal,path:&path,offset:0,max_bytes:8192,cancel:None,chunk_bytes:4096};
 let before=std::fs::read_to_string(&log).unwrap().matches("FUSE_FETCH_DATA=").count();
 let before_open=std::fs::read_to_string(&log).unwrap().matches("FUSE_DATA_OPEN").count();
 let safe=if digest {
  match engine.digest_bounded(&request,&ConservativeProbe,&policy){
   Ok(value)=>value.bytes_digested==0 && !value.confirmed() && value.stopped==Some(InspectionStop::Placeholder),
   Err(EngineError::Business(BusinessError::Unsupported))=>true,
   other=>{eprintln!("actual digest result: {other:?}");false}
  }
 }else{
  match engine.read_bounded(&request,&ConservativeProbe,&policy){
   Ok(value)=>{eprintln!("actual read bytes={} stop={:?}",value.bytes.len(),value.stopped);value.bytes.is_empty() && value.stopped==Some(InspectionStop::Placeholder)},
   Err(EngineError::Business(BusinessError::Unsupported))=>true,
   other=>{eprintln!("actual read result: {other:?}");false}
  }
 };
 let after=std::fs::read_to_string(&log).unwrap().matches("FUSE_FETCH_DATA=").count();
 eprintln!("DG_ACTUAL_FUSE_PRODUCT digest={digest} before={before} after={after} safe={safe}");
 let after_open=std::fs::read_to_string(&log).unwrap().matches("FUSE_DATA_OPEN").count();
 assert_eq!(after_open,before_open,"product requested a data handle on unknown provider");
 assert_eq!(after,before,"product triggered real content callback");
 assert!(safe,"product must refuse unknown no-recall capability");
}
