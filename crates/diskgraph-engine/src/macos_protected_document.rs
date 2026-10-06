use crate::EngineError;
use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_installation_lease::open_namespace;
use diskgraph_core::BusinessError;
use std::os::unix::fs::FileExt;
use std::time::Instant;

/// 根句柄约束的root保护配置读取，不从普通环境建立信任或向外导出文件句柄。
/// 来源：原生 Rust PF-06 活跃安装配置合同；无 Java 对等对象。
pub(super) struct MacosProtectedDocument;

impl MacosProtectedDocument {
    /// 参数：path仅供内部固定配置定位，cap最多64KiB，检查点和期限沿原请求。
    /// 返回：身份稳定的有界原文件字节；不跟随任何组件链接，不授予执行权限。
    pub(super) fn read(
        path: &[u8],
        cap: usize,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Vec<u8>, EngineError> {
        check(deadline, checkpoint)?;
        if cap == 0 || cap > 64 * 1024 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let (ancestors, file, state) = open_namespace(path, deadline, checkpoint)?;
        if state.len > cap as u64 {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let length = state.len as usize;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| BusinessError::BudgetExceeded)?;
        bytes.resize(length, 0);
        let mut offset = 0;
        while offset < length {
            check(deadline, checkpoint)?;
            let read = match file.read_at(&mut bytes[offset..], offset as u64) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            check(deadline, checkpoint)?;
            if read == 0 {
                return Err(BusinessError::Conflict.into());
            }
            offset += read;
        }
        check(deadline, checkpoint)?;
        let mut excess = [0_u8; 1];
        let read = file.read_at(&mut excess, state.len)?;
        check(deadline, checkpoint)?;
        let current_file = MacosFilesystemState::capture(&file, false)?;
        check(deadline, checkpoint)?;
        if read != 0 || current_file != state {
            return Err(BusinessError::Conflict.into());
        }
        for (directory, original) in &ancestors {
            check(deadline, checkpoint)?;
            let current_directory = MacosFilesystemState::capture(directory, true)?;
            check(deadline, checkpoint)?;
            if !original.same_directory_binding(&current_directory) {
                return Err(BusinessError::Conflict.into());
            }
        }
        // 只用新打开句柄复核名称绑定；内容始终来自最初的原FD。
        let (current, _comparison, current_state) = open_namespace(path, deadline, checkpoint)?;
        if current_state != state
            || current.len() != ancestors.len()
            || current
                .iter()
                .zip(&ancestors)
                .any(|((_, now), (_, original))| !original.same_directory_binding(now))
        {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, checkpoint)?;
        Ok(bytes)
    }
}
fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
