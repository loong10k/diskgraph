#[cfg(test)]
mod gated_reader;
#[cfg(test)]
mod hooks;
#[cfg(test)]
mod tests;

use crate::{FrameReader, worker_failure::WorkerFailure, worker_request::WorkerRequest};
use diskgraph_disktree_core::scan::ScanProgress;
use std::io::{self, Read};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

/// 控制管道线程的唯一句柄 owner；来源：std::thread 与 pinned ScanProgress::cancel。
/// 阻塞的部分输入由父进程终止协议处理，不把此线程 join 当作上游 walker 的 join。
pub(crate) struct WorkerControl {
    thread: Option<JoinHandle<()>>,
    progress: Arc<ScanProgress>,
    failure: Arc<Mutex<Option<io::Error>>>,
    terminal: Arc<AtomicBool>,
}

impl WorkerControl {
    /// 参数：reader 为已消费 Request 的原输入账本，progress 为本次真实扫描的原 Arc。
    /// 返回：已拥有控制线程的 owner；spawn 失败取消原扫描并返回真实错误。
    pub(crate) fn start<R: Read + Send + 'static>(
        mut reader: FrameReader<R>,
        progress: Arc<ScanProgress>,
    ) -> io::Result<Self> {
        let failure = Arc::new(Mutex::new(None));
        let terminal = Arc::new(AtomicBool::new(false));
        let error_slot = Arc::clone(&failure);
        let is_terminal = Arc::clone(&terminal);
        let signal = Arc::clone(&progress);
        #[cfg(test)]
        let (read_returned, published) = hooks::take_reader_witnesses();
        let result = thread::Builder::new()
            .name("scan-worker-control".into())
            .spawn(move || {
                loop {
                    let error = match reader.read_payload::<WorkerRequest>() {
                        Ok(Some(WorkerRequest::Cancel {})) => {
                            signal.cancel();
                            continue;
                        }
                        Ok(None) if is_terminal.load(Ordering::Acquire) => break,
                        Ok(None) => io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "control closed before terminal output",
                        ),
                        Ok(Some(WorkerRequest::Request { .. })) => io::Error::new(
                            io::ErrorKind::InvalidData,
                            "second Request is forbidden",
                        ),
                        Err(error) => error,
                    };
                    #[cfg(test)]
                    if let Some(read_returned) = &read_returned {
                        let _ = read_returned.send(());
                    }
                    *error_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
                    signal.cancel();
                    #[cfg(test)]
                    if let Some(published) = &published {
                        let _ = published.send(());
                    }
                    break;
                }
            });
        match result {
            Ok(thread) => Ok(Self {
                thread: Some(thread),
                progress,
                failure,
                terminal,
            }),
            Err(error) => {
                progress.cancel();
                Err(error)
            }
        }
    }

    /// 参数：self 为本次扫描控制器。
    /// 返回：真实协议错误或已观察到的取消；绝不把取消后的部分树当成功。
    pub(crate) fn check(&self) -> Result<(), WorkerFailure> {
        // 在同一错误槽锁内观察取消，避免真实协议错误发布后被误分类为用户取消。
        let mut failure = self.failure.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(error) = failure.take() {
            return Err(WorkerFailure::new("protocol", error));
        }
        #[cfg(test)]
        hooks::after_empty(&self.failure);
        let cancelled = self.progress.is_cancelled();
        drop(failure);
        if cancelled {
            return Err(WorkerFailure::new(
                "cancelled",
                io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"),
            ));
        }
        Ok(())
    }

    /// 参数：self 为即将发送 End 或 Error 的控制器。
    /// 返回：无返回值；此后干净 stdin EOF 仅结束控制 reader，不再产生新失败。
    pub(crate) fn mark_terminal(&self) {
        self.terminal.store(true, Ordering::Release);
    }

    /// 参数：self 为持有真实控制线程句柄的 owner，父端在终帧后应关闭控制管道。
    /// 返回：真实 join 结果；不证明 pinned scanner 的物理线程退出。
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("control reader panicked"))?;
        }
        Ok(())
    }
}

impl Drop for WorkerControl {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.progress.cancel();
            let _ = self.finish();
        }
    }
}
