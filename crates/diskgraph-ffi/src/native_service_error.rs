/// 原生会话初始化或生命周期错误；对应平台宿主的 typed error。
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum NativeServiceError {
    /// 数据库无法打开、迁移失败或会话已经关闭。
    #[error("{reason}")]
    Unavailable { reason: String },
}
