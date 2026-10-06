/// 原始 OVERLAPPED 的操作种类。来源：Win32 连接与读取完成语义；原生 Rust，无 Java 对等对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WindowsPipeIoPhase {
    /// 尚未提交操作，允许初始化连接。
    Prepared,
    /// 已完成连接，允许提交读取。
    Ready,
    /// 已提交连接；完成或取消均不能解释为读取 EOF。
    Connecting,
    /// 已提交读取；仅读取完成路径解释内容与 EOF。
    Reading,
}
