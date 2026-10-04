//! 控制准备字段在 ValueRef 借用阶段复用原会话；来源：Rust D42，不创建独立预算 owner。
use crate::{ControlStore, Result, StoreError};
use diskgraph_core::{JobRequestAuthority, ProcessEvidenceJobInput, ProcessEvidenceLimits};

impl ControlStore {
    /// 参数：真实 Process job；返回：流式数字限额、原始读取成本及固定 bootstrap 分配准入量。
    /// 此唯一 bootstrap 不拥有输入/epoch/范围；16KiB 硬门禁先执行，随后立即计入原认领会话。
    /// 返回的限额不是执行资格；消费者仍须同账本完整重验 typed 输入/原 authority/实时 fence。
    pub fn process_job_limits_with_cost(
        &self,
        job_id: &str,
    ) -> Result<(ProcessEvidenceLimits, u64, u64)> {
        crate::process_job_input_codec::with_raw(&self.connection, job_id, |raw, _, _, _, bytes| {
            let limits = crate::process_job_limits_view::ProcessJobLimitsView::decode(raw)?;
            // 数值 visitor 无 Value 树；包含 escaped field-name scratch 的固定服务器上界。
            let allocation = raw
                .len()
                .checked_mul(2)
                .and_then(|n| n.checked_add(1024))
                .ok_or(StoreError::BudgetExceeded)?;
            Ok((
                limits,
                u64::try_from(bytes).map_err(|_| StoreError::BudgetExceeded)?,
                u64::try_from(allocation).map_err(|_| StoreError::BudgetExceeded)?,
            ))
        })
    }

    /// 参数：真实job和同会话(raw,entries,allocation)回调；返回：拥有前准入且完整验证的原输入。
    pub fn process_evidence_job_input_with_admission(
        &self,
        job_id: &str,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<ProcessEvidenceJobInput> {
        crate::process_job_input_codec::read_with_admission(&self.connection, job_id, admit)
    }

    /// 参数：真实job和同会话回调；返回：已验证原请求身份或明确旧来源未知，不含bearer。
    pub fn job_request_authority_with_admission(
        &self,
        job_id: &str,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<Option<JobRequestAuthority>> {
        crate::job_authority_gate::read_with_admission(&self.connection, job_id, admit)
    }
}
