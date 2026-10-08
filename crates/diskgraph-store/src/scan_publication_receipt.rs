use crate::{JobKind, JobRecord, Result, StoreError};
use diskgraph_core::{JobRequestAuthority, PrincipalId, ScopeId, ServerId};
use serde::{Deserialize, Serialize};

/// 保存一次扫描已经提交的事实，不授予新的扫描或内容权限；来源：原生 Rust RT-01。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanPublicationReceipt {
    pub(crate) job_id: String,
    pub(crate) kind: JobKind,
    pub(crate) principal: PrincipalId,
    pub(crate) server_id: ServerId,
    pub(crate) scope_id: ScopeId,
    pub(crate) authority: Option<JobRequestAuthority>,
    pub(crate) publishing_fence: u64,
    pub(crate) snapshot_id: String,
    pub(crate) revision_id: String,
    pub(crate) published_at_unix_ms: u64,
}

impl ScanPublicationReceipt {
    /// 参数：原认领任务、真实服务器、原持久身份、结果快照/revision 和发布时间；返回：有限提交事实或拒绝。
    pub fn new(
        job: &JobRecord,
        server_id: ServerId,
        authority: Option<JobRequestAuthority>,
        snapshot_id: String,
        revision_id: String,
        published_at_unix_ms: u64,
    ) -> Result<Self> {
        let receipt = Self {
            job_id: job.job_id.clone(),
            kind: job.kind,
            principal: job.principal.clone(),
            server_id,
            scope_id: job.scope_id.clone(),
            authority,
            publishing_fence: job.fencing_token,
            snapshot_id,
            revision_id,
            published_at_unix_ms,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// 参数：无；返回：原始任务标识，不按新 fence 或 latest 推算。
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    /// 参数：无；返回：已提交 revision 的真实标识。
    pub fn revision_id(&self) -> &str {
        &self.revision_id
    }
    /// 参数：无；返回：已提交扫描快照的真实标识。
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }
    /// 参数：无；返回：实际发布代次，不能用作新认领权限。
    pub fn publishing_fence(&self) -> u64 {
        self.publishing_fence
    }

    /// 参数：无；返回：结构合法且编码有限；不对已提交事实重新要求原 token 未到期。
    pub(crate) fn validate(&self) -> Result<()> {
        if !matches!(self.kind, JobKind::Index | JobKind::Sync)
            || [&self.job_id, &self.snapshot_id, &self.revision_id]
                .iter()
                .any(|v| v.is_empty() || v.len() > 256)
            || self.publishing_fence == 0
            || self.publishing_fence > i64::MAX as u64
            || self.published_at_unix_ms > i64::MAX as u64
            || self
                .authority
                .as_ref()
                .is_some_and(|a| a.principal() != &self.principal)
        {
            return Err(StoreError::InvalidGraph(
                "invalid scan publication receipt".into(),
            ));
        }
        if serde_json::to_vec(self)?.len() > 16384 {
            return Err(StoreError::InvalidGraph(
                "oversized scan publication receipt".into(),
            ));
        }
        Ok(())
    }
}
