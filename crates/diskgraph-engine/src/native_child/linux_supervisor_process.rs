use super::ChildSpawnError;
use super::linux_atomic_child::LinuxAtomicChild;
use crate::recovery_control::{LinuxControlRead, LinuxControlReader};
use crate::recovery_slot::{ActiveSlot, LinuxSupervisorMaterials, LinuxSupervisorTrust};
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// 原监督子进程、私有 IPC 与全局 ACTIVE 槽的唯一父侧 owner；来源：PF-06 root broker 合同。
/// Drop 不写 CLEAN；原私有绝对期限保持不变，尚不构成长运行 MCP 或安装服务资格。
#[must_use = "retain original process and bootstrap until authenticated retirement"]
pub struct LinuxSupervisorProcess {
    pub(super) child: Option<Box<LinuxAtomicChild>>,
    pub(super) materials: Option<LinuxSupervisorMaterials>,
    pub(super) controls: Option<LinuxControlReader>,
    pub(super) born: bool,
    control_eof: bool,
    retired_child: bool,
    slot: Option<ActiveSlot>,
}

impl LinuxSupervisorProcess {
    /// 参数：slot 为已同步 ACTIVE 的全局原锁；返回：出生前预分配的完整处置槽。
    pub(super) fn before_birth(slot: ActiveSlot) -> Self {
        Self {
            child: None,
            materials: None,
            controls: None,
            born: false,
            control_eof: false,
            retired_child: false,
            slot: Some(slot),
        }
    }

    /// 参数：trust 为宿主原部署句柄，cancel 为原取消；返回：一次有界原材料交付结果。
    pub fn send_materials(
        &mut self,
        trust: &LinuxSupervisorTrust,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        self.materials
            .as_mut()
            .ok_or_else(|| io::Error::other("original materials sender unavailable"))?
            .send(trust, cancel)
            .map_err(io::Error::other)
    }

    /// 参数：cancel 为原取消；返回：逐消息内核认证的通知，EOF 不代替原 child wait。
    pub fn poll_notification(&mut self, cancel: &AtomicBool) -> io::Result<LinuxControlRead> {
        let read = self
            .controls
            .as_mut()
            .ok_or_else(|| io::Error::other("original control reader unavailable"))?
            .poll(cancel)
            .map_err(io::Error::other)?;
        if matches!(&read, LinuxControlRead::ConfirmedEof) {
            self.control_eof = true;
        }
        Ok(read)
    }

    /// 参数：无；返回：至多4096字节借用 stdout；调用方须公平排空两输出端。
    pub fn read_stdout(&mut self) -> io::Result<Option<&[u8]>> {
        match self.child.as_mut() {
            Some(child) => child.read_stdout().map_err(io::Error::other),
            None => Ok(None),
        }
    }

    /// 参数：无；返回：至多4096字节借用 stderr；不加载完整输出或刷新请求预算。
    pub fn read_stderr(&mut self) -> io::Result<Option<&[u8]>> {
        match self.child.as_mut() {
            Some(child) => child.read_stderr().map_err(io::Error::other),
            None => Ok(None),
        }
    }

    /// 参数：deadline 为本次原绝对期限；返回：原通知、输出 EOF、真实 wait 与槽同步全成立才为 true。
    /// 先用 poll_notification 完成私有流；普通退出或未确认控制不能释放全局容量。
    pub fn poll_retirement(&mut self, deadline: Instant) -> io::Result<bool> {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "original retirement deadline",
            ));
        }
        if self.slot.is_none() {
            return Ok(true);
        }
        if !self.control_eof {
            return Ok(false);
        }
        if !self.retired_child {
            let child = self
                .child
                .as_mut()
                .ok_or_else(|| io::Error::other("original child unavailable"))?;
            child.request_control_close().map_err(io::Error::other)?;
            let mut checkpoint = || {
                if Instant::now() < deadline {
                    Ok(())
                } else {
                    Err(())
                }
            };
            match child.poll_normal_exit(&mut checkpoint) {
                Ok(false) => return Ok(false),
                Ok(true) => self.retired_child = true,
                Err(ChildSpawnError::Operation(error)) => return Err(io::Error::other(error)),
                Err(ChildSpawnError::Checkpoint { .. }) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "original retirement checkpoint",
                    ));
                }
            }
            // 原 wait 已消费，才关闭本轮输出和私有端点；槽写入失败仍保留原 ACTIVE owner。
            self.child.take();
            self.controls.take();
            self.materials.take();
        }
        self.slot
            .as_mut()
            .expect("original bootstrap retained")
            .confirm_original_cleanup(deadline)
            .map_err(io::Error::other)?;
        self.slot.take();
        Ok(true)
    }

    /// 参数：deadline 为原单次恢复期限；返回：异常停止原 child 的进度，绝不据此清除 ACTIVE。
    pub fn poll_cleanup(&mut self, deadline: Instant) -> io::Result<bool> {
        match self.child.as_mut() {
            Some(child) => child.poll_cleanup(deadline).map_err(io::Error::other),
            None => Ok(!self.born || self.retired_child),
        }
    }

    /// 参数：deadline 为原取消期限；返回：仅实际未出生时可显式释放全局原槽。
    pub fn abort_unborn(&mut self, deadline: Instant) -> io::Result<()> {
        if self.born {
            return Err(io::Error::other(
                "born supervisor cannot use prebirth abort",
            ));
        }
        if let Some(slot) = self.slot.as_mut() {
            slot.confirm_original_cleanup(deadline)
                .map_err(io::Error::other)?;
            self.slot.take();
        }
        Ok(())
    }
}
