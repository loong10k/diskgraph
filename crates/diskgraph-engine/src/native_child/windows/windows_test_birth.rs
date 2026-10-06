//! 原生夹具的有限出生与失败救援；产品只保留调用方拥有槽位的出生接口。
use super::WindowsChild;
use crate::native_child::{ChildInputMode, ChildSpawnError};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::Command;
use std::time::{Duration, Instant};

/// 真实Windows夹具的限时启动适配；来源：原生测试，不是产品兼容入口。
pub(crate) struct WindowsTestBirth;

impl WindowsTestBirth {
    /// 参数：原夹具命令及生命周期检查点；返回：实际Null子进程或原失败。
    pub(crate) fn spawn<E>(
        command: &mut Command,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<WindowsChild, ChildSpawnError<E>> {
        Self::spawn_with_input(command, ChildInputMode::Null, checkpoint)
    }

    /// 参数：原命令、管道用途与生命周期检查；返回：真实owner或保留原值的失败。
    /// 夹具在catch外持有owner，panic先沿原对象救援再恢复原payload。
    pub(crate) fn spawn_with_input<E>(
        command: &mut Command,
        mode: ChildInputMode,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<WindowsChild, ChildSpawnError<E>> {
        let mut owner = None;
        let result = catch_unwind(AssertUnwindSafe(|| {
            Self::spawn_into(command, mode, &mut owner, checkpoint)
        }));
        match result {
            Ok(Ok(())) => Ok(owner.take().expect("actual successful fixture birth")),
            Ok(Err(error)) => {
                let cleanup = owner.as_mut().map_or(Ok(()), WindowsChild::cleanup);
                Err(match error {
                    ChildSpawnError::Operation(primary) => {
                        ChildSpawnError::Operation(primary.with_cleanup(cleanup))
                    }
                    ChildSpawnError::Checkpoint {
                        primary,
                        cleanup: prior,
                    } => ChildSpawnError::Checkpoint {
                        primary,
                        cleanup: match (prior, cleanup) {
                            (None, Ok(())) => None,
                            (None, Err(error)) => Some(error),
                            (Some(error), cleanup) => Some(error.with_cleanup(cleanup)),
                        },
                    },
                })
            }
            Err(payload) => {
                if let Some(child) = owner.as_mut() {
                    let _ = child.cleanup();
                }
                resume_unwind(payload)
            }
        }
    }

    /// 参数：catch外原槽及原生命周期检查；返回：原结果，失败/panic仍由调用方持有owner。
    /// 独立20秒救援门只约束夹具，不改变产品期限或原四阶段检查次数。
    pub(crate) fn spawn_into<E>(
        command: &mut Command,
        mode: ChildInputMode,
        owner: &mut Option<WindowsChild>,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<(), ChildSpawnError<E>> {
        let deadline = Instant::now() + Duration::from_secs(20);
        WindowsChild::spawn_into_with_admission(
            command,
            mode,
            owner,
            || {
                assert!(Instant::now() < deadline, "fixture birth admission expired");
                Ok(())
            },
            checkpoint,
        )
    }
}
