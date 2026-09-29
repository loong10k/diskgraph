//! Local identity bootstrap for the single-user CLI: publishes policy v1 if
//! needed, grants the well-known local principal server administration, and
//! rebuilds the authorizer from the control store on every run.

use diskgraph_core::{PolicyAuthorizer, PrincipalId};
use diskgraph_engine::{Engine, EngineError};

pub const LOCAL_PRINCIPAL: &str = "local-user";

/// Loaded once per CLI invocation.
pub struct LocalIdentity;

impl LocalIdentity {
    /// Idempotently bootstraps the local admin principal and returns the
    /// authorizer as it stands in the control store; decisions still filter
    /// by the acting principal at call time.
    pub fn load(engine: &Engine) -> Result<PolicyAuthorizer, EngineError> {
        let local = PrincipalId::new(LOCAL_PRINCIPAL.to_owned())
            .map_err(|_| EngineError::Business(diskgraph_core::BusinessError::InternalError))?;
        engine.bootstrap_local_admin(&local)?;
        engine.policy_authorizer()
    }
}
