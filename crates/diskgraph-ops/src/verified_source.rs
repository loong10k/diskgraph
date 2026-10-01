use crate::bound_path::BoundPath;

/// 复制验证期间固定的源目录、文件句柄与元数据，删除源时必须再次匹配。
pub(crate) struct VerifiedSource {
    pub(crate) path: BoundPath,
    pub(crate) file: std::fs::File,
    pub(crate) metadata: std::fs::Metadata,
}
