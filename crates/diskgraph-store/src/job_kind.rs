use diskgraph_core::Permission;
use serde::{Deserialize, Serialize};

/// 首次索引或同步的持久任务类型。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// What kind of durable work a job represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Index,
    Sync,
    GitEvidence,
    ProcessEvidence,
}

impl JobKind {
    /// 编码或解析既有稳定 wire 标签。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：任务类型或生命周期对应的 snake_case 标签。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Sync => "sync",
            Self::GitEvidence => "git_evidence",
            Self::ProcessEvidence => "process_evidence",
        }
    }

    /// 编码或解析既有稳定 wire 标签。
    /// 参数：value：待编码/解析字段。
    /// 返回：已知任务标签对应枚举，未知为 None。
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "index" => Some(Self::Index),
            "sync" => Some(Self::Sync),
            "git_evidence" => Some(Self::GitEvidence),
            "process_evidence" => Some(Self::ProcessEvidence),
            _ => None,
        }
    }

    /// 参数：无；返回：持久任务类型不可降低的权限，旧入口亦必须执行。
    pub fn required_permissions(self) -> &'static [Permission] {
        match self {
            Self::Index | Self::Sync => &[Permission::IndexWrite],
            Self::ProcessEvidence => &[Permission::MetadataRead, Permission::IndexWrite],
            Self::GitEvidence => &[
                Permission::MetadataRead,
                Permission::IndexWrite,
                Permission::ContentRead,
            ],
        }
    }
}
