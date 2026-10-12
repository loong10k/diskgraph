use std::io;

/// 原guardian可信启动后的永久专用身份切换；来源：PF-06，无Java对等对象。
/// 不创建Engine、不读取客户文件；失败不得恢复root或据此声明服务准入成功。
pub(crate) struct LinuxGuardianIdentity;

impl LinuxGuardianIdentity {
    /// 参数：uid/gid为root安装配置的专用身份；返回：三组身份、零组/能力、NNP与隐私复核成功。
    /// 仅无业务线程的原启动入口使用；调用前须保留原executor、私有通道和容量槽。
    pub(crate) fn enter(uid: u32, gid: u32) -> io::Result<()> {
        let result =
            unsafe { super::linux_supervisor_abi::dg_linux_guardian_drop_identity(uid, gid) };
        if result < 0 {
            return Err(io::Error::from_raw_os_error((-result) as i32));
        }
        Ok(())
    }
}
