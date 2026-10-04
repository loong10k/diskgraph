//! 不保留访问历史的稳定匿名标签规则。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

/// --anonymize 将根替换为固定标签，并按节点 ID 为子节点生成稳定匿名名。
/// 无需保留所有已访问节点；大小、结构和分类保持精确。
/// 来源：DiskGraph 原生 Rust tui::Pseudonyms；无 Java 对应对象。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pseudonyms;

impl Pseudonyms {
    /// 创建无状态的稳定匿名标签规则。
    /// 参数：无。返回：按 revision 内节点 ID 生成标签的规则。
    pub fn new() -> Self {
        Self
    }

    /// 返回可分享的根目录标签。
    /// 参数：无。返回：固定的 home 标签。
    pub fn root(&self) -> &'static str {
        "home"
    }

    /// 按 revision 内节点 ID 生成稳定标签，页面重访和下钻不保留额外映射。
    /// 参数：node_id 为真实节点标识。返回：稳定匿名标签，大小和结构不改变。
    pub(super) fn label_for(&self, node_id: u64) -> String {
        format!("dir-{node_id:02}")
    }
}
