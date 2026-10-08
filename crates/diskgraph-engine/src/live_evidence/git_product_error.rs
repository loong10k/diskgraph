use super::probe_failure::ProbeFailure;
use diskgraph_core::BusinessError;

/// 产品 Git 边界的有界类型化结果；来源：实际资源门禁和 ProbeFailure，不回传原始工具文本。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitProductError {
    Budget,
    Deadline,
    Cancelled,
    IdentityChanged,
    Unavailable,
    Unverifiable,
}

impl GitProductError {
    /// 从内部已锁存类型生成产品分类。参数：failure 为原始类型；返回：不包含路径、命令或正文的类别。
    pub(super) fn from_failure(failure: Option<&ProbeFailure>) -> Self {
        match failure {
            Some(ProbeFailure::Deadline) => Self::Deadline,
            Some(ProbeFailure::Cancelled) => Self::Cancelled,
            Some(ProbeFailure::OutputLimit | ProbeFailure::ResourceLimit) => Self::Budget,
            Some(ProbeFailure::IdentityChanged) => Self::IdentityChanged,
            Some(
                ProbeFailure::Io(_)
                | ProbeFailure::AbnormalExit(_)
                | ProbeFailure::CommandExit { .. },
            ) => Self::Unavailable,
            Some(ProbeFailure::Cleanup { primary, .. }) => Self::from_failure(Some(primary)),
            Some(ProbeFailure::InvalidLimits | ProbeFailure::Unsupported(_)) | None => {
                Self::Unverifiable
            }
        }
    }

    /// 映射既有稳定业务错误。参数：无；返回：有界业务类别，不能把失败表示成 clean。
    pub(crate) fn business(self) -> BusinessError {
        match self {
            Self::Budget => BusinessError::BudgetExceeded,
            Self::Deadline => BusinessError::Timeout,
            Self::Cancelled | Self::IdentityChanged => BusinessError::Conflict,
            Self::Unavailable => BusinessError::Unavailable,
            Self::Unverifiable => BusinessError::Unsupported,
        }
    }
}
