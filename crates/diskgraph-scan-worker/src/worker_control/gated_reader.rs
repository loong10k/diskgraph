use std::io::{self, Read};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// 可确定释放的真实 Read 错误源；来源：std::io::Read，不伪造 WorkerFailure 或取消标志。
pub(super) struct GatedReader {
    pub(super) gate: Receiver<()>,
    pub(super) ready: Sender<()>,
    pub(super) kind: io::ErrorKind,
}

impl Read for GatedReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        let _ = self.ready.send(());
        self.gate
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("test reader gate not released: {error}"),
                )
            })?;
        Err(io::Error::new(self.kind, "qualified control Read failure"))
    }
}
