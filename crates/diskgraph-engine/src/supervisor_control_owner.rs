use crate::recovery_control::{ControlError, ControlNotification, UnixControlWriter};
use crate::{SupervisorControlProgress, SupervisorOwner, SupervisorRecoveryError};
use diskgraph_core::BusinessError;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// 同时持有原退休责任与唯一私有 writer 的监督接线；来源：PF-06，无 Java 对等对象。
/// 不认证对端，不生成出生资格；调用方须从已认证出生路径移交原未克隆通道。
#[must_use = "retain the original owner despite control failure or pending retirement"]
pub struct SupervisorControlOwner {
    owner: SupervisorOwner,
    writer: UnixControlWriter,
    ready_attempted: bool,
    foreground_ended: bool,
    pending_attempted: bool,
    completion_attempted: bool,
    control_failure: Option<ControlError>,
}

impl SupervisorControlOwner {
    /// 参数：owner 为已绑定的原 Engine/Recovery/ACTIVE，writer 为原私有单向控制能力。
    /// 返回：保持两者完整寿命的接线；不发送消息，也不克隆或转移原 pending 内存。
    pub fn new(owner: SupervisorOwner, writer: UnixControlWriter) -> Self {
        Self {
            owner,
            writer,
            ready_attempted: false,
            foreground_ended: false,
            pending_attempted: false,
            completion_attempted: false,
            control_failure: None,
        }
    }

    /// 参数：无；返回：前台结束前的原 Engine 引用，结束后不再提供业务入口。
    /// 借用方若保留原 Arc，真实退休仍 Pending；本方法不复制执行能力或刷新期限。
    pub fn engine(&self) -> Option<&std::sync::Arc<crate::Engine>> {
        if self.foreground_ended {
            None
        } else {
            self.owner.engine()
        }
    }

    /// 参数：cancel 为原控制取消；返回：Ready 已完整发送或锁存的原控制错误。
    /// 只能尝试一次；失败仍持有全部原资源，调用方不得开始前台业务。
    pub fn ready(&mut self, cancel: &AtomicBool) -> Result<(), ControlError> {
        if self.ready_attempted || self.foreground_ended {
            return self.fail(ControlError::Protocol);
        }
        self.ready_attempted = true;
        self.send(ControlNotification::Ready {}, cancel)
    }

    /// 参数：exit_code/panicked 为实际前台结果，cancel 为原控制取消；返回：通知交付或原失败。
    /// 调用方须先停止前台与 join runner；通知失败也记录前台结束，以继续处置原资源。
    pub fn foreground_ended(
        &mut self,
        exit_code: i32,
        panicked: bool,
        cancel: &AtomicBool,
    ) -> Result<(), ControlError> {
        if self.foreground_ended {
            return self.fail(ControlError::Protocol);
        }
        self.foreground_ended = true;
        if !self.ready_attempted {
            return self.fail(ControlError::Protocol);
        }
        self.send(
            ControlNotification::ForegroundEnded {
                exit_code,
                panicked,
            },
            cancel,
        )
    }

    /// 参数：deadline 为原单次恢复期限，cancel 仅取消通知，不取消原恢复责任。
    /// 返回：实际 Pending、原退休且已写完成通知、或原退休但控制未确认；原资源错误原样返回。
    /// 只在前台结束后推进；重复轮询最多发送一次 Pending 和一次 Complete，不刷新 writer 原期限。
    pub fn poll_retirement(
        &mut self,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<SupervisorControlProgress, SupervisorRecoveryError> {
        if !self.foreground_ended {
            return Err(crate::EngineError::from(BusinessError::Conflict).into());
        }
        // 控制断线/超时不能阻断同一原 owner 的实际退休；不能用通道成功代替物理回收。
        if !self.owner.poll_retirement(deadline)? {
            if !self.pending_attempted {
                self.pending_attempted = true;
                let _ = self.send(ControlNotification::CleanupPending {}, cancel);
            }
            return Ok(SupervisorControlProgress::Pending {
                control_failure: self.control_failure,
            });
        }
        if !self.completion_attempted {
            self.completion_attempted = true;
            let _ = self.send(ControlNotification::CleanupComplete {}, cancel);
        }
        Ok(match self.control_failure {
            Some(control_failure) => {
                SupervisorControlProgress::RetiredUnconfirmed { control_failure }
            }
            None => SupervisorControlProgress::Complete,
        })
    }

    fn send(
        &mut self,
        notification: ControlNotification,
        cancel: &AtomicBool,
    ) -> Result<(), ControlError> {
        if let Some(error) = self.control_failure {
            return Err(error);
        }
        match self.writer.send(notification, cancel) {
            Ok(()) => Ok(()),
            Err(error) => self.fail(error),
        }
    }

    fn fail(&mut self, error: ControlError) -> Result<(), ControlError> {
        let original = *self.control_failure.get_or_insert(error);
        Err(original)
    }
}
