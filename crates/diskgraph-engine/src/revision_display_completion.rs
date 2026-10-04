/// 可信 TUI 导航或已绘制画布的完成状态，不携带任意查询数据或另一份授权状态。
/// 来源：DiskGraph 原生 Rust Q-08 / 13.6；无 Java 对应对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionDisplayCompletion {
    /// 已完成的导航页或画布；图读取期限过后不得提交。
    Complete,
    /// 已绘制保留父块并显示真实截断原因的部分画布。
    Truncated,
}
