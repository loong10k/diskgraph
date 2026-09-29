//! Capability model and default-deny authorization (P0 task 1.4, specs
//! SC-03 / SC-05 / OP-01).
//!
//! Every capability is separate: metadata access never implies content access,
//! and no query capability implies any file action. The default authorizer
//! denies everything, and approvals are never a permission a client can hold.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::ids::{PrincipalId, ScopeId};

/// One file-level mutation capability; each action is granted independently.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileActionKind {
    Move,
    Copy,
    Trash,
    Restore,
    Purge,
}

/// A single grantable capability. There is deliberately no `Approve` variant:
/// approvals come from a trusted channel, not from a client-held permission.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read indexes, coverage, sizes, and relations for a scope (M).
    MetadataRead,
    /// Read bounded file contents; never implied by metadata access (C).
    ContentRead,
    /// Create and manage indexes for a scope (I).
    IndexWrite,
    /// Register or remove scopes; server administration (S).
    ScopeAdmin,
    /// View or cancel durable operations for a scope (O).
    OperationView,
    /// One concrete file action (F); each kind is its own capability.
    FileAction(FileActionKind),
}

impl Permission {
    /// Stable wire name, e.g. `files:trash`, used across CLI/MCP/FFI.
    pub fn wire_name(self) -> String {
        match self {
            Self::MetadataRead => "metadata:read".into(),
            Self::ContentRead => "content:read".into(),
            Self::IndexWrite => "index:write".into(),
            Self::ScopeAdmin => "scope:admin".into(),
            Self::OperationView => "operations:view".into(),
            Self::FileAction(kind) => format!(
                "files:{}",
                match kind {
                    FileActionKind::Move => "move",
                    FileActionKind::Copy => "copy",
                    FileActionKind::Trash => "trash",
                    FileActionKind::Restore => "restore",
                    FileActionKind::Purge => "purge",
                }
            ),
        }
    }
}

/// One grant: this principal may use this permission in this scope as of this
/// policy version. Grants from superseded policy versions stop applying.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    pub principal: PrincipalId,
    pub permission: Permission,
    pub scope: ScopeId,
    pub policy_version: u64,
}

/// The outcome of one authorization decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Allowed,
    Denied(DenyReason),
}

/// Why a request was denied; stable codes for tests and diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DenyReason {
    /// No deployment has enabled this capability at all (OP-01 default).
    Disabled,
    /// No grant covers this principal + permission + scope.
    NoMatchingGrant,
    /// A matching grant exists under a superseded policy version.
    PolicyVersionMismatch,
    /// The whole policy was revoked; nothing is authorized.
    PolicyRevoked,
}

/// The authorization seam every entry point consults before doing work.
pub trait Authorizer {
    fn decide(&self, principal: &PrincipalId, permission: &Permission, scope: &ScopeId)
    -> Decision;

    /// The policy epoch this authorizer decides under. Cursors bind to it so
    /// a policy update invalidates pages started under the old epoch (P4-5.9).
    /// Static authorizers report 0, which never mismatches itself.
    fn policy_version(&self) -> u64 {
        0
    }
}

/// The default: deny everything. A deployment that never grants capabilities
/// stays read-only-by-omission, satisfying OP-01 without extra flags.
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyAllAuthorizer;

impl Authorizer for DenyAllAuthorizer {
    fn decide(
        &self,
        _principal: &PrincipalId,
        _permission: &Permission,
        _scope: &ScopeId,
    ) -> Decision {
        Decision::Denied(DenyReason::Disabled)
    }
}

/// A versioned grant set. Publishing a new version invalidates every grant
/// recorded under older versions; revocation denies all principals at once.
#[derive(Clone, Debug, Default)]
pub struct PolicyAuthorizer {
    current_version: u64,
    revoked: bool,
    grants: HashSet<Grant>,
}

impl PolicyAuthorizer {
    pub fn new(current_version: u64) -> Self {
        Self {
            current_version,
            revoked: false,
            grants: HashSet::new(),
        }
    }

    /// Adds a grant bound to the current policy version.
    pub fn grant(
        &mut self,
        principal: PrincipalId,
        permission: Permission,
        scope: ScopeId,
    ) -> &mut Self {
        let version = self.current_version;
        self.grant_at_version(principal, permission, scope, version);
        self
    }

    /// Adds a grant pinned to an explicit version; used when rebuilding the
    /// authorizer from durable storage. Versions newer than the current one
    /// cannot exist and are ignored.
    pub fn grant_at_version(
        &mut self,
        principal: PrincipalId,
        permission: Permission,
        scope: ScopeId,
        policy_version: u64,
    ) {
        if policy_version <= self.current_version {
            self.grants.insert(Grant {
                principal,
                permission,
                scope,
                policy_version,
            });
        }
    }

    /// Publishes a new policy version; grants pinned to older versions expire.
    pub fn publish_version(&mut self, version: u64) {
        self.current_version = version;
    }

    /// Revokes the policy; nothing is authorized until a new version is published.
    pub fn revoke(&mut self) {
        self.revoked = true;
    }

    /// Restores service after revocation by re-publishing a version.
    pub fn republish(&mut self, version: u64) {
        self.revoked = false;
        self.current_version = version;
    }

    pub fn current_version(&self) -> u64 {
        self.current_version
    }
}

impl Authorizer for PolicyAuthorizer {
    fn policy_version(&self) -> u64 {
        self.current_version
    }

    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        if self.revoked {
            return Decision::Denied(DenyReason::PolicyRevoked);
        }
        let probe = Grant {
            principal: principal.clone(),
            permission: *permission,
            scope: scope.clone(),
            policy_version: self.current_version,
        };
        if self.grants.contains(&probe) {
            return Decision::Allowed;
        }
        let stale = self.grants.iter().any(|grant| {
            grant.principal == *principal
                && grant.permission == *permission
                && grant.scope == *scope
        });
        if stale {
            Decision::Denied(DenyReason::PolicyVersionMismatch)
        } else {
            Decision::Denied(DenyReason::NoMatchingGrant)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(name: &str) -> PrincipalId {
        PrincipalId::new(name).unwrap()
    }

    fn scope(name: &str) -> ScopeId {
        ScopeId::new(name).unwrap()
    }

    fn allowed(decision: Decision) -> bool {
        decision == Decision::Allowed
    }

    #[test]
    fn default_authorizer_denies_everything_including_reads() {
        let authorizer = DenyAllAuthorizer;
        for permission in [
            Permission::MetadataRead,
            Permission::ContentRead,
            Permission::IndexWrite,
            Permission::ScopeAdmin,
            Permission::OperationView,
            Permission::FileAction(FileActionKind::Trash),
        ] {
            assert_eq!(
                authorizer.decide(&principal("agent"), &permission, &scope("project")),
                Decision::Denied(DenyReason::Disabled)
            );
        }
    }

    #[test]
    fn metadata_grant_does_not_imply_content_index_or_file_actions() {
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(
            principal("agent"),
            Permission::MetadataRead,
            scope("project"),
        );
        let authorizer = &policy;
        assert!(allowed(authorizer.decide(
            &principal("agent"),
            &Permission::MetadataRead,
            &scope("project")
        )));
        for permission in [
            Permission::ContentRead,
            Permission::IndexWrite,
            Permission::ScopeAdmin,
            Permission::OperationView,
        ] {
            assert_eq!(
                authorizer.decide(&principal("agent"), &permission, &scope("project")),
                Decision::Denied(DenyReason::NoMatchingGrant),
                "{permission:?} must not ride along with metadata:read"
            );
        }
    }

    #[test]
    fn each_file_action_is_its_own_capability() {
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(
            principal("agent"),
            Permission::FileAction(FileActionKind::Move),
            scope("project"),
        );
        let authorizer = &policy;
        assert!(allowed(authorizer.decide(
            &principal("agent"),
            &Permission::FileAction(FileActionKind::Move),
            &scope("project")
        )));
        assert_eq!(
            authorizer.decide(
                &principal("agent"),
                &Permission::FileAction(FileActionKind::Trash),
                &scope("project")
            ),
            Decision::Denied(DenyReason::NoMatchingGrant)
        );
        assert_eq!(
            authorizer.decide(
                &principal("agent"),
                &Permission::FileAction(FileActionKind::Purge),
                &scope("project")
            ),
            Decision::Denied(DenyReason::NoMatchingGrant)
        );
    }

    #[test]
    fn grants_do_not_cross_scopes_or_principals() {
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(
            principal("agent"),
            Permission::MetadataRead,
            scope("project"),
        );
        let authorizer = &policy;
        assert_eq!(
            authorizer.decide(
                &principal("agent"),
                &Permission::MetadataRead,
                &scope("other")
            ),
            Decision::Denied(DenyReason::NoMatchingGrant)
        );
        assert_eq!(
            authorizer.decide(
                &principal("human"),
                &Permission::MetadataRead,
                &scope("project")
            ),
            Decision::Denied(DenyReason::NoMatchingGrant)
        );
    }

    #[test]
    fn publishing_a_new_policy_version_expires_old_grants() {
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(
            principal("agent"),
            Permission::MetadataRead,
            scope("project"),
        );
        policy.publish_version(2);
        assert_eq!(
            policy.decide(
                &principal("agent"),
                &Permission::MetadataRead,
                &scope("project")
            ),
            Decision::Denied(DenyReason::PolicyVersionMismatch)
        );
        policy.grant(
            principal("agent"),
            Permission::MetadataRead,
            scope("project"),
        );
        assert!(allowed(policy.decide(
            &principal("agent"),
            &Permission::MetadataRead,
            &scope("project")
        )));
    }

    #[test]
    fn revocation_denies_everyone_until_republish() {
        let mut policy = PolicyAuthorizer::new(3);
        policy.grant(
            principal("agent"),
            Permission::MetadataRead,
            scope("project"),
        );
        policy.revoke();
        assert_eq!(
            policy.decide(
                &principal("agent"),
                &Permission::MetadataRead,
                &scope("project")
            ),
            Decision::Denied(DenyReason::PolicyRevoked)
        );
        policy.republish(4);
        assert_eq!(
            policy.decide(
                &principal("agent"),
                &Permission::MetadataRead,
                &scope("project")
            ),
            Decision::Denied(DenyReason::PolicyVersionMismatch)
        );
        policy.grant(
            principal("agent"),
            Permission::MetadataRead,
            scope("project"),
        );
        assert!(allowed(policy.decide(
            &principal("agent"),
            &Permission::MetadataRead,
            &scope("project")
        )));
    }

    #[test]
    fn wire_names_are_stable_across_entry_points() {
        assert_eq!(Permission::MetadataRead.wire_name(), "metadata:read");
        assert_eq!(Permission::ContentRead.wire_name(), "content:read");
        assert_eq!(
            Permission::FileAction(FileActionKind::Trash).wire_name(),
            "files:trash"
        );
        assert_eq!(
            Permission::FileAction(FileActionKind::Purge).wire_name(),
            "files:purge"
        );
    }
}
