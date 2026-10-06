use crate::native_child::ChildError;
use crate::scan_worker_child::ScanWorkerChild as DriverChild;
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_input::ScanWorkerInput;
use crate::scan_worker_output::ScanWorkerOutput;
use crate::scan_worker_owned_failure::ScanWorkerOwnedFailure;
use diskgraph_scan_worker::{ExecutionOutcome, ScanProgress, WorkerRequest};
use std::convert::Infallible;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::Instant;

/// 独占真实 Child 的非阻塞父端驱动，不把 End／EOF／进度完成当 OS 退出。
/// 来源：PF-06；原请求期限／授权检查贯穿控制、双输出、协议组装和正常 wait。
pub(crate) struct ScanWorkerDriver {
    child: Option<DriverChild>,
    input: ScanWorkerInput,
    output: ScanWorkerOutput,
    deadline: Instant,
    stderr_first: bool,
    stopped: bool,
    completed: bool,
    unwind_cleanup: Option<ChildError>,
}

impl ScanWorkerDriver {
    /// 参数：child 为调用者在恢复边界外持有的唯一 owner 槽，request 为唯一 v2 DTO，expected 为声明匹配值，
    /// deadline 为原 Instant，stderr_limit 是原响应总额内部子限额。
    /// 返回：有限配置的驱动或原配置错误；失败及配置 panic 时 owner 留在调用者槽。
    /// 成功后才移交 owner；失败由调用者显式处置和保留清理错误，不授权 helper 文件或续租。
    pub(crate) fn new(
        child: &mut Option<DriverChild>,
        request: WorkerRequest,
        expected: (&str, &str),
        deadline: Instant,
        stderr_limit: u64,
    ) -> Result<Self, ScanWorkerFailure<Infallible>> {
        let configured: Result<_, ScanWorkerFailure<Infallible>> = (|| {
            ScanWorkerFailure::check(deadline, &mut || Ok::<(), Infallible>(()))?;
            let limits = match &request {
                WorkerRequest::Request {
                    version: 2, limits, ..
                } => limits.checked()?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected one execution Request version 2",
                    )
                    .into());
                }
            };
            let output = ScanWorkerOutput::new(limits, expected, stderr_limit)?;
            let input = ScanWorkerInput::new(&request)?;
            ScanWorkerFailure::check(deadline, &mut || Ok::<(), Infallible>(()))?;
            Ok((input, output))
        })();
        let (input, output) = configured?;
        let child = child.take().ok_or_else(|| {
            ScanWorkerFailure::from(io::Error::new(
                io::ErrorKind::InvalidData,
                "scan worker owner missing",
            ))
        })?;
        // 到此配置已经全部完成；移交后仅构造 owner，不再执行可失败的准备。

        Ok(Self {
            child: Some(child),
            input,
            output,
            deadline,
            stderr_first: false,
            stopped: false,
            completed: false,
            unwind_cleanup: None,
        })
    }

    /// 参数：request_cancel 为真实 caller 请求，checkpoint 借原授权／fence／取消检查。
    /// 返回：至多一次完整协议且真实退出的结果；None 仅 Pending，原 E 与清理原因不改写。
    /// 正常 poll 不等待，原生异常清理可能等待；不承诺硬20ms或 RSS 上界。
    pub(crate) fn poll<E>(
        &mut self,
        request_cancel: bool,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Option<ExecutionOutcome>, ScanWorkerFailure<E>> {
        if self.stopped {
            return Err(ScanWorkerFailure::Stopped {
                cleanup: self.unwind_cleanup.take(),
            });
        }
        if self.completed {
            return Ok(None);
        }
        // 检查函数或 native 后处理 panic 时，借用 self 的 unwind 也必须实际清理；
        // 不能只依赖调用者最终 Drop。原 panic payload 原样继续展开。
        match catch_unwind(AssertUnwindSafe(|| {
            self.poll_checked(request_cancel, &mut checkpoint)
        })) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => {
                self.stopped = true;
                Err(error.with_cleanup(
                    self.child
                        .as_mut()
                        .expect("driver owns its child")
                        .cleanup(),
                ))
            }
            Err(payload) => {
                self.stopped = true;
                self.unwind_cleanup = self
                    .child
                    .as_mut()
                    .expect("driver owns its child")
                    .cleanup()
                    .err();
                resume_unwind(payload)
            }
        }
    }

    fn poll_checked<E>(
        &mut self,
        request_cancel: bool,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Option<ExecutionOutcome>, ScanWorkerFailure<E>> {
        ScanWorkerFailure::check(self.deadline, checkpoint)?;
        self.input.request_cancel(request_cancel);
        self.pump_input(checkpoint)?;
        // 每轮各读取一个固定块，起始管道轮转；stderr 回压不能饿死 Request 或 stdout。
        self.stderr_first = !self.stderr_first;
        if self.stderr_first {
            self.read_stderr(checkpoint)?;
            self.read_stdout(checkpoint)?;
        } else {
            self.read_stdout(checkpoint)?;
            self.read_stderr(checkpoint)?;
        }
        if self.output.terminal() {
            self.input.begin_close();
            self.pump_input(checkpoint)?;
        }
        if self
            .child
            .as_ref()
            .expect("driver owns its child")
            .stdout_eof()
        {
            self.output
                .finish(&mut || ScanWorkerFailure::check(self.deadline, checkpoint))?;
            if self.output.tree() && self.input.cancelled() {
                return Err(ScanWorkerFailure::Cancelled { cleanup: None });
            }
        }
        if !self
            .child
            .as_ref()
            .expect("driver owns its child")
            .stdout_eof()
            || !self
                .child
                .as_ref()
                .expect("driver owns its child")
                .stderr_eof()
            || !self.input.closed()
        {
            return Ok(None);
        }
        let exited = self
            .child
            .as_mut()
            .expect("driver owns its child")
            .poll_normal_exit(&mut || ScanWorkerFailure::check(self.deadline, checkpoint))
            .map_err(ScanWorkerFailure::from_child)?;
        if !exited {
            return Ok(None);
        }
        if self.output.tree()
            && self
                .child
                .as_ref()
                .expect("driver owns its child")
                .exit_code()
                != Some(0)
        {
            return Err(ScanWorkerFailure::Exit {
                code: self
                    .child
                    .as_ref()
                    .expect("driver owns its child")
                    .exit_code(),
                cleanup: None,
            });
        }
        ScanWorkerFailure::check(self.deadline, checkpoint)?;
        self.completed = true;
        self.output.take().map(Some).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "missing assembled execution outcome",
            )
            .into()
        })
    }

    fn pump_input<E>(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), ScanWorkerFailure<E>> {
        ScanWorkerFailure::check(self.deadline, checkpoint)?;
        if self.input.closed() {
            return Ok(());
        }
        if self.input.closing() {
            let status = if self.input.close_polling() {
                self.child
                    .as_mut()
                    .expect("driver owns its child")
                    .poll_control_write()?
            } else {
                self.child
                    .as_mut()
                    .expect("driver owns its child")
                    .request_control_close()?
            };
            self.input.observe_close(status)?;
        } else if self.input.pending() {
            let status = self
                .child
                .as_mut()
                .expect("driver owns its child")
                .poll_control_write()?;
            self.input.observe(status, 0)?;
        } else if let Some(chunk) = self.input.chunk()? {
            let length = chunk.len();
            let status = self
                .child
                .as_mut()
                .expect("driver owns its child")
                .start_control_write(chunk)?;
            self.input.observe(status, length)?;
        }
        ScanWorkerFailure::check(self.deadline, checkpoint)
    }

    fn read_stdout<E>(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), ScanWorkerFailure<E>> {
        ScanWorkerFailure::check(self.deadline, checkpoint)?;
        if !self
            .child
            .as_ref()
            .expect("driver owns its child")
            .stdout_eof()
            && let Some(bytes) = self
                .child
                .as_mut()
                .expect("driver owns its child")
                .read_stdout()?
        {
            self.output.stdout(bytes, &mut || {
                ScanWorkerFailure::check(self.deadline, checkpoint)
            })?;
        }
        ScanWorkerFailure::check(self.deadline, checkpoint)
    }

    fn read_stderr<E>(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), ScanWorkerFailure<E>> {
        ScanWorkerFailure::check(self.deadline, checkpoint)?;
        if !self
            .child
            .as_ref()
            .expect("driver owns its child")
            .stderr_eof()
            && let Some(bytes) = self
                .child
                .as_mut()
                .expect("driver owns its child")
                .read_stderr()?
        {
            self.output.stderr(bytes.len())?;
        }
        ScanWorkerFailure::check(self.deadline, checkpoint)
    }

    /// 参数：无；返回：最后一次真实 Progress 全字段的借用；messages 原样，finished 不授予退出。
    pub(crate) fn progress(&self) -> Option<&ScanProgress> {
        self.output.progress()
    }

    /// 参数：无；返回：已观察 leader 的真实正常退出码；None 也可表示未退出或信号退出。
    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.child
            .as_ref()
            .expect("driver owns its child")
            .exit_code()
    }

    /// 参数：消费驱动及本次原 poll 失败；返回：显式处置结果和必要的原 owner。
    /// 此入口只处理通常的错误移交，panic 的外层宿主处置槽仍须单独接入和验收。
    pub(crate) fn into_owned_failure<E>(
        mut self,
        error: ScanWorkerFailure<E>,
    ) -> ScanWorkerOwnedFailure<E> {
        self.stopped = true;
        let child = self
            .child
            .take()
            .expect("driver owns its child before transfer");
        ScanWorkerOwnedFailure::dispose(error, child)
    }

    /// 参数：无；返回：原 panic 已继续展开后的实际 cleanup 失败，未解析或覆盖原 payload。
    pub(crate) fn unwind_cleanup_error(&self) -> Option<&ChildError> {
        self.unwind_cleanup.as_ref()
    }
}

impl Drop for ScanWorkerDriver {
    fn drop(&mut self) {
        if !self.completed {
            // 已显式移交时本对象不再拥有 child；最后兜底不把 cleanup 当正常退出许可。
            if let Some(child) = self.child.as_mut() {
                let _ = child.cleanup();
            }
        }
    }
}
