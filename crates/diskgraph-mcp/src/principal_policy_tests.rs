//! 已认证请求的窄策略仍与 token 能力和实时数据库授权相交。
use crate::{auth, protocol::ToolProfile};
use diskgraph_core::{Authorizer, Decision, Grant, Permission};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn narrow_request_policy_keeps_subject_capability_and_revocation_boundaries() {
    let (service, _data) = crate::tests::service(ToolProfile::ReadFull, "principal-policy");
    let root = tempfile::tempdir().unwrap();
    let local = service.context.principal();
    let scope = service
        .engine()
        .register_scope(root.path(), local, &service.authorizer().unwrap())
        .unwrap();
    let authenticator =
        auth::Authenticator::new(auth::AuthConfig::single("issuer", "aud", b"fixture-key"));
    let token = auth::TokenMinter::new(b"fixture-key").mint(&auth::TokenClaims {
        issuer: "issuer".into(),
        audience: "aud".into(),
        subject: "remote-reader".into(),
        expires_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300,
        scope: Some("metadata:read".into()),
    });
    let identity = authenticator.authenticate(Some(&token)).unwrap();
    {
        let mut control = service.engine().control_store().unwrap();
        let version = control.policy_version().unwrap();
        for permission in [Permission::MetadataRead, Permission::ContentRead] {
            control
                .upsert_grant(&Grant {
                    principal: identity.principal.clone(),
                    permission,
                    scope: scope.clone(),
                    policy_version: version,
                })
                .unwrap();
        }
    }
    let remote = service.for_identity(&identity);
    let policy = remote
        .authorizer_until(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        policy.decide(&identity.principal, &Permission::MetadataRead, &scope),
        Decision::Allowed
    );
    assert_ne!(
        policy.decide(&identity.principal, &Permission::ContentRead, &scope),
        Decision::Allowed
    );
    assert_ne!(
        policy.decide(local, &Permission::MetadataRead, &scope),
        Decision::Allowed
    );
    service
        .engine()
        .control_store()
        .unwrap()
        .revoke_grant(&identity.principal, &Permission::MetadataRead, &scope)
        .unwrap();
    let policy = remote
        .authorizer_until(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert_ne!(
        policy.decide(&identity.principal, &Permission::MetadataRead, &scope),
        Decision::Allowed
    );
}
