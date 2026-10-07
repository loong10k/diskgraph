//! 查询及任务回归使用产品可信宿主，禁止以内存扫描或种树替代真实前置。
use super::CliEngineHost;
use diskgraph_core::BusinessError;
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use std::ops::Deref;

/// 保留产品宿主及外部恢复责任的测试生命周期；来源：原生 Rust PF-06，无 Java 对等对象。
/// 未配置可信扫描宿主时明确失败；不跳过原测试，不扩大请求权限。
pub(crate) struct CliTestEngine {
    host: Option<CliEngineHost>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _capacity_directory: tempfile::TempDir,
}

impl CliTestEngine {
    /// 参数：config 为原测试的隔离目录及预算；返回：真实产品宿主或原准入错误。
    pub(crate) fn open(config: EngineConfig) -> Result<Self, EngineError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let capacity_directory = {
            use std::os::unix::fs::PermissionsExt;
            let directory = tempfile::tempdir()?;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
            directory
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let host = CliEngineHost::open_in_test_domain(
            config,
            std::fs::File::open(capacity_directory.path())?,
        )?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let host = CliEngineHost::open(config)?;
        if host.recovery.is_none() {
            return Err(BusinessError::Unsupported.into());
        }
        Ok(Self {
            host: Some(host),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            _capacity_directory: capacity_directory,
        })
    }
}

impl Deref for CliTestEngine {
    type Target = Engine;
    fn deref(&self) -> &Self::Target {
        &self.host.as_ref().expect("test host remains owned").engine
    }
}

impl Drop for CliTestEngine {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            // 原命令恢复边界继续实际排空同一 registry，不把测试结束当成回收完成。
            // 兼容恢复可等待，不据此声明产品已具有有限退出能力。
            host.execute(|_| Ok(())).expect("actual test host recovery");
        }
    }
}
