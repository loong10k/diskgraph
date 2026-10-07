/// 独立宿主提供的监督域及 user/mount 命名空间身份。来源：PF-06；无 Java 对等对象。
/// 不从待准入目录生成信任；产品必须从可信宿主启动材料读取，不能采用 TOFU 或远程值。
/// 本类型只封装预期身份，不验证发行签名或授予监督进程出生资格。
pub struct LinuxSupervisorTrust {
    pub(super) root: File,
    pub(super) user_namespace: File,
    pub(super) mount_namespace: File,
}

impl LinuxSupervisorTrust {
    /// 参数：root/user_namespace/mount_namespace 为独立宿主持有并交付的原对象句柄。
    /// 返回：保持原对象寿命的部署材料；不得从待核路径自建原句柄作为 TOFU。
    /// 不接受长期裸 inode 数字，避免对象释放及 inode 重用使陈旧材料重新匹配。
    pub fn from_host(root: File, user_namespace: File, mount_namespace: File) -> Self {
        Self {
            root,
            user_namespace,
            mount_namespace,
        }
    }
}

/// 读取原文件句柄的内核身份，不通过路径重新打开。
/// 参数：file 为持有的原句柄；返回：设备号与 inode，或原元数据读取错误。
pub(super) fn identity(file: &File) -> Result<(u64, u64), super::SlotError> {
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
use std::fs::File;
use std::os::unix::fs::MetadataExt;
