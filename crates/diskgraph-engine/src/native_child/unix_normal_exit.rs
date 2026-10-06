use super::ChildError;
use super::unix_leader::UnixLeader;

/// 仅 fresh 私有 session 可领取正常回收许可，完成事实不再依赖数值 PID/PGID。
/// 来源：原生 Rust OS-child 的 setsid、retained leader 与正常 wait 生命周期。
#[derive(Default)]
pub(super) enum UnixNormalExit {
    #[default]
    Unavailable,
    Qualified,
    Completed,
}

impl UnixNormalExit {
    /// 查询正常退出资格。参数：无；返回：真实 qualified 路径时为 true。
    pub(super) fn is_qualified(&self) -> bool {
        !matches!(self, Self::Unavailable | Self::Completed)
    }

    /// 查询已经回收的缓存事实。参数：无；返回：不再访问旧数值身份的完成状态。
    pub(super) fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// 保留旧平台资格拒绝。参数：无；返回：对应真实正常退出能力或明确 Unsupported。
    pub(super) fn validate_platform(&self) -> Result<(), ChildError> {
        #[cfg(target_os = "macos")]
        if matches!(self, Self::Qualified) {
            return Ok(());
        }
        Err(ChildError::Unsupported(
            "complete normal process-group view is unavailable on this Unix platform",
        ))
    }

    /// 查询整组退出事实。参数：pid 仅供原 Mac retained group；返回：活动为 false，未知为错误。
    pub(super) fn group_exited(&self, pid: u32) -> Result<bool, ChildError> {
        #[cfg(target_os = "macos")]
        if matches!(self, Self::Qualified) {
            return match super::macos_child_group::normal_view(pid) {
                super::macos_group_view::MacosGroupView::Active => Ok(false),
                super::macos_group_view::MacosGroupView::AllExited => Ok(true),
                super::macos_group_view::MacosGroupView::Unknown(error) => Err(error),
            };
        }
        #[cfg(not(target_os = "macos"))]
        let _ = pid;
        Err(ChildError::Unsupported(
            "child has no complete normal exit evidence",
        ))
    }

    /// 消费已经满足全退出证据的原 child。参数：child 为原唯一句柄；返回：实际 wait 或原 OS 错误。
    pub(super) fn reap(&self, child: &mut UnixLeader) -> Result<(), ChildError> {
        child
            .wait()
            .map(|_| ())
            .map_err(|error| ChildError::io("normally reap owned child", error))
    }
}
