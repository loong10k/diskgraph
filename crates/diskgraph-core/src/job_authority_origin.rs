use serde::{Deserialize, Serialize};

/// 区分显式可信本地请求和经过传输认证的远程请求；缺失持久记录不属于任何来源。
/// 来源：DiskGraph 原生 Rust SC-06 持久请求授权设计，无 Java 对应对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobAuthorityOrigin {
    /// 可信 CLI、stdio 或内部兼容调用，仍须服从实际范围与已发布策略。
    TrustedLocal,
    /// 已认证远程请求，能力上限与原始绝对到期时间均不可缺失。
    AuthenticatedRemote,
}
