//! macOS 原 fresh-session owner 的单次异常回收，不重置预算或丢失原等待责任。
use super::super::ChildError;
use super::super::unix_child_group::UnixChildGroup;
use super::super::unix_normal_exit::UnixNormalExit;
use super::UnixChild;
use std::io;
use std::time::Instant;

impl UnixChild {
    /// 参数：deadline 为宿主原绝对期限；返回：实际完整回收 true，活动/到期 false，原错误保留。
    /// 只接受 fresh 私有 session；原兼容入口保留旧 cleanup，不借此提升正常退出资格。
    /// 单次 OS 查询及短状态操作不承诺硬墙钟上限；不 sleep、不阻塞 wait、不新建 owner。
    pub(crate) fn poll_cleanup(&mut self, deadline: Instant) -> Result<bool, ChildError> {
        if self.cleaned {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        if !self.owns_group {
            return Err(ChildError::Unsupported(
                "child ownership lost; refusing numeric process-group cleanup",
            ));
        }
        if !self.normal_exit.is_qualified() {
            return Err(ChildError::Unsupported(
                "bounded cleanup requires original private-session qualification",
            ));
        }
        #[cfg(test)]
        if self.cleanup_fault {
            self.cleanup_fault = false;
            return Err(ChildError::Io("injected cleanup failure".into()));
        }
        let finished = self.poll()?;
        if Instant::now() >= deadline {
            return Ok(false);
        }
        let pid = i32::try_from(self.child.id())
            .map_err(|_| ChildError::Unsupported("unrepresentable child pid"))?;
        // 终止组失败不得消费 leader；它仍是下一次原组重试的身份锚点。
        UnixChildGroup::terminate_once(pid)?;
        if Instant::now() >= deadline {
            return Ok(false);
        }
        if !finished && unsafe { libc::kill(pid, libc::SIGKILL) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(ChildError::io("terminate owned child", error));
            }
        }
        if Instant::now() >= deadline || !self.poll()? {
            return Ok(false);
        }
        if !self.normal_exit.group_exited(self.child.id())? {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        #[cfg(test)]
        super::super::unix_normal_exit_tests::reap_before_cleanup_wait(pid);
        match self.child.poll_wait() {
            Ok(None) => return Ok(false),
            Ok(Some(_)) => {}
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.owns_group = false;
                }
                return Err(ChildError::io("reap owned child", error));
            }
        }
        // 完成事实先保存，末段期限不能抹掉实际已消费的原 wait。
        self.cleaned = true;
        self.owns_group = false;
        self.normal_exit = UnixNormalExit::Completed;
        self.stdout.take();
        self.stderr.take();
        self.control.take();
        Ok(true)
    }
}
