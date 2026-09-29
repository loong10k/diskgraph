//! Ports that keep the engine decoupled from platforms and optional tools
//! (P0 task 1.6, specs EC-01 / PF-03).
//!
//! Concrete implementations arrive with later stages; what P0 fixes is the
//! contract: providers declare what they can do, capability gaps surface as
//! `unsupported`, and no port ever shells out to `ls`/`du`/`find` or assumes a
//! native path where a provider URI is the truth.

use serde::{Deserialize, Serialize};

use crate::errors::BusinessError;
use crate::ids::ScopeId;
use crate::permissions::FileActionKind;

/// Which observation substrate backs a scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Direct native filesystem access (macOS/Linux/Windows).
    NativeFs,
    /// Host-mediated documents: Android SAF URIs, iOS security-scoped documents.
    DocumentProvider,
}

/// How confident a provider can be about reported sizes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeCapability {
    /// Sizes are unavailable (unknown stays null, never zero).
    Unknown,
    /// Sizes are observable for enumerable resources.
    Observable,
}

/// What one provider can do. Every false here must surface as `unsupported`
/// downstream instead of a degraded surprise.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub enumerate: bool,
    pub sizes: SizeCapability,
    pub read_content: bool,
    pub trash: bool,
    pub restore: bool,
}

impl ProviderCapabilities {
    /// The metadata-only baseline: enumerate and sizes, nothing else.
    pub const METADATA_ONLY: Self = Self {
        enumerate: true,
        sizes: SizeCapability::Observable,
        read_content: false,
        trash: false,
        restore: false,
    };

    /// What a full native filesystem provider aims to offer (per platform validation).
    pub const NATIVE_FULL: Self = Self {
        enumerate: true,
        sizes: SizeCapability::Observable,
        read_content: true,
        trash: true,
        restore: true,
    };
}

/// Provider-level operations whose support can be probed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderOperation {
    Enumerate,
    ReadContent,
    Trash,
    Restore,
}

/// Turns a capability probe into the contract's `unsupported` error, so a
/// metadata-only provider can never drift into content or mutation work.
pub fn capability_decision(
    capabilities: &ProviderCapabilities,
    operation: ProviderOperation,
) -> Result<(), BusinessError> {
    let supported = match operation {
        ProviderOperation::Enumerate => capabilities.enumerate,
        ProviderOperation::ReadContent => capabilities.read_content,
        ProviderOperation::Trash => capabilities.trash,
        ProviderOperation::Restore => capabilities.restore,
    };
    if supported {
        Ok(())
    } else {
        Err(BusinessError::Unsupported)
    }
}

/// The provider seam every scanner and host integration implements.
pub trait ResourceProvider {
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> ProviderCapabilities;
}

/// Sizes and free space come from platform volume APIs, never from `df` parsing.
pub trait VolumeMeter {
    /// `Ok(None)` means "not measurable here"; callers must keep it unknown.
    fn free_bytes(&self, scope: &ScopeId) -> Result<Option<u64>, BusinessError>;
}

/// Evidence producers declare their identity so provenance is testable.
pub trait EvidenceCollectorPort {
    fn collector_id(&self) -> &str;
    fn version(&self) -> u32;
}

/// The P5 execution seam; declared now so ops work cannot invent a shell bridge.
pub trait FileOperatorPort {
    /// Actions this platform operator can perform; missing actions stay `unsupported`.
    fn supported_actions(&self) -> &'static [FileActionKind];
}

/// Approval verification is a separate trust boundary (OP-03); operators check,
/// they never mint approvals.
pub trait ApprovalVerifierPort {
    fn verify(&self, approval_ref: &str, plan_digest: &str) -> ApprovalDecision;
}

/// Outcome of checking one approval against one plan digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    Valid,
    Invalid,
    Expired,
}

/// Port for scheduling post-operation index refreshes without an ops→engine
/// dependency cycle (technical design §1).
pub trait RefreshSchedulerPort {
    fn request_refresh(&self, scope: &ScopeId);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DocumentUri-style provider: enumerate + sizes only, no content, no mutation.
    struct MetadataOnlyProvider;

    impl ResourceProvider for MetadataOnlyProvider {
        fn kind(&self) -> ProviderKind {
            ProviderKind::DocumentProvider
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::METADATA_ONLY
        }
    }

    #[test]
    fn provider_kind_and_capabilities_declare_metadata_only_scope() {
        let provider = MetadataOnlyProvider;
        assert_eq!(provider.kind(), ProviderKind::DocumentProvider);
        assert_eq!(provider.capabilities(), ProviderCapabilities::METADATA_ONLY);
    }

    #[test]
    fn metadata_only_provider_reports_unsupported_for_content_and_mutations() {
        let capabilities = ProviderCapabilities::METADATA_ONLY;
        assert_eq!(
            capability_decision(&capabilities, ProviderOperation::Enumerate),
            Ok(())
        );
        assert_eq!(
            capability_decision(&capabilities, ProviderOperation::ReadContent),
            Err(BusinessError::Unsupported)
        );
        assert_eq!(
            capability_decision(&capabilities, ProviderOperation::Trash),
            Err(BusinessError::Unsupported)
        );
        assert_eq!(
            capability_decision(&capabilities, ProviderOperation::Restore),
            Err(BusinessError::Unsupported)
        );
    }

    #[test]
    fn native_full_capabilities_cover_every_probed_operation() {
        let capabilities = ProviderCapabilities::NATIVE_FULL;
        for operation in [
            ProviderOperation::Enumerate,
            ProviderOperation::ReadContent,
            ProviderOperation::Trash,
            ProviderOperation::Restore,
        ] {
            assert_eq!(capability_decision(&capabilities, operation), Ok(()));
        }
    }

    #[test]
    fn unknown_volume_space_is_none_not_zero() {
        struct UnknownMeter;
        impl VolumeMeter for UnknownMeter {
            fn free_bytes(&self, _scope: &ScopeId) -> Result<Option<u64>, BusinessError> {
                Ok(None)
            }
        }
        let scope = ScopeId::new("project").unwrap();
        assert_eq!(UnknownMeter.free_bytes(&scope).unwrap(), None);
    }

    #[test]
    fn collector_identity_is_declared_for_provenance() {
        struct CargoProjectCollector;
        impl EvidenceCollectorPort for CargoProjectCollector {
            fn collector_id(&self) -> &str {
                "cargo-projects"
            }
            fn version(&self) -> u32 {
                1
            }
        }
        let collector = CargoProjectCollector;
        assert_eq!(collector.collector_id(), "cargo-projects");
        assert_eq!(collector.version(), 1);
    }
}
