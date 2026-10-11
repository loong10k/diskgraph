use super::CliEngineHost;
use diskgraph_core::BusinessError;
use diskgraph_engine::recovery_control::UnixControlWriter;
use diskgraph_engine::{
    EngineError, SupervisorControlOwner, SupervisorControlProgress, SupervisorOwner,
    SupervisorParts,
};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

impl CliEngineHost {
    /// 参数：control 为原已认证出生通道，deadline 为原构造期限；返回：原绑定及 Ready，或保留材料的失败。
    pub(super) fn attach_control(
        &mut self,
        control: UnixControlWriter,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        let parts = SupervisorParts {
            engine: Arc::clone(&self.engine),
            scan: self.recovery.take(),
            slot: self.slot.take().expect("original admitted supervisor slot"),
        };
        let owner = match SupervisorOwner::bind(parts, deadline) {
            Ok(owner) => owner,
            Err(parts) => {
                self.recovery = parts.scan;
                self.slot = Some(parts.slot);
                return Err(BusinessError::RecoveryUnconfirmed.into());
            }
        };
        let mut owner = SupervisorControlOwner::new(owner, control);
        let ready = if owner.is_lifecycle() {
            owner.ready_lifecycle(&AtomicBool::new(false), deadline)
        } else {
            owner.ready(&AtomicBool::new(false))
        };
        self.control_owner = Some(owner);
        // 即使 Ready 写入失败也先保存原 owner；失败沿原 execute 恢复而不是丢弃原 ACTIVE。
        ready.map_err(|_| BusinessError::RecoveryUnconfirmed.into())
    }

    /// 参数：outcome 为实际前台结果或原 panic；返回：原结果，控制失败不伪装成完成，也不丢原恢复责任。
    pub(super) fn retire_controlled_supervisor<T>(
        self,
        outcome: std::thread::Result<Result<T, EngineError>>,
    ) -> Result<T, EngineError> {
        let _namespace_guard = self.namespace_guard;
        let mut owner = self.control_owner.expect("original controlled owner");
        // 前台栈已真实结束，先释放 host 的同一 Arc，才能尝试原 owner 的独占退休。
        drop(self.engine);
        let (exit_code, panicked) = match &outcome {
            Ok(Ok(_)) => (0, false),
            Ok(Err(error)) => (
                i32::from(crate::output::engine_business(error).exit_code()),
                false,
            ),
            Err(_) => (101, true),
        };
        let cancel = AtomicBool::new(false);
        let notification = if owner.is_lifecycle() {
            owner.foreground_ended_lifecycle(
                exit_code,
                panicked,
                &cancel,
                Instant::now() + Duration::from_millis(50),
            )
        } else {
            owner.foreground_ended(exit_code, panicked, &cancel)
        };
        let mut reported = notification.is_err();
        if reported {
            eprintln!("supervisor private control unconfirmed; original recovery retained");
        }
        loop {
            match owner.poll_retirement(Instant::now() + Duration::from_millis(50), &cancel) {
                Ok(SupervisorControlProgress::Complete) => break,
                Ok(SupervisorControlProgress::RetiredUnconfirmed { .. }) => {
                    eprintln!(
                        "supervisor original resources retired; private completion unconfirmed"
                    );
                    break;
                }
                Ok(SupervisorControlProgress::Pending { control_failure }) => {
                    if control_failure.is_some() && !reported {
                        eprintln!(
                            "supervisor private control unconfirmed; original recovery retained"
                        );
                        reported = true;
                    }
                }
                Err(_) if !reported => {
                    eprintln!("supervisor original resource retirement pending");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
