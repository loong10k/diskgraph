use serde::{Deserialize, Serialize};

/// 可持久的已知失败阶段，Execution 包含整个执行而非推断子步骤；来源：原生 Rust EC-02 / RT-03。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitEvidenceFailurePhase {
    Admission,
    Execution,
    Publication,
    Reconciliation,
}
impl GitEvidenceFailurePhase {
    /// 参数：无；返回：固定安全标签，不能包含路径或程序输出。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::Execution => "execution",
            Self::Publication => "publication",
            Self::Reconciliation => "reconciliation",
        }
    }
}
