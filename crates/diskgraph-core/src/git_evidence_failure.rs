use crate::{GitEvidenceFailureCode, GitEvidenceFailurePhase};
use serde::{Deserialize, Serialize};

/// 断线后可查询的有限失败诊断，仅固定阶段/业务码；来源：原生 Rust EC-02 / RT-03。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitEvidenceFailure {
    phase: GitEvidenceFailurePhase,
    code: GitEvidenceFailureCode,
}
impl GitEvidenceFailure {
    /// 参数：实际已知阶段与 typed 类别；返回：无原始路径、秘密和程序文本的诊断。
    pub fn new(phase: GitEvidenceFailurePhase, code: GitEvidenceFailureCode) -> Self {
        Self { phase, code }
    }
    /// 参数：无；返回：实际已知阶段。
    pub fn phase(&self) -> GitEvidenceFailurePhase {
        self.phase
    }
    /// 参数：无；返回：固定业务类别。
    pub fn code(&self) -> GitEvidenceFailureCode {
        self.code
    }
}
