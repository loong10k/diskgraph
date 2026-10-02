//! 仅提供占位对象诊断，不替代平台原生策略与句柄访问门禁。

use std::path::Path;

/// 仅提供占位对象诊断，不替代平台原生策略与句柄访问门禁。
/// 来源：原生 Rust diskgraph-engine::content::PlaceholderProbe。
/// Detects a cloud placeholder. Real platforms need platform API surface
/// (macOS dataless-flag stat, Windows reparse cloud attributes); the
/// 此探针只是诊断；实际内容访问必须执行平台原生策略及句柄检查。
/// macOS 禁止 dataless 物化，Windows 暴露占位属性；真实 provider 不下载另行验收。
/// Tests inject fakes, which never replace the native access guard.
pub trait PlaceholderProbe {
    /// 参数：path 为待诊断路径。
    /// 返回：是否观察到占位特征；不能替代原生访问门禁。
    fn is_placeholder(&self, path: &Path) -> bool;
}
