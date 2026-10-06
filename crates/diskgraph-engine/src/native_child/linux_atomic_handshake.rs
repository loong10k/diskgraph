use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_message::LinuxAtomicMessage;
use super::{ChildError, ChildSpawnError};
use std::io;
use std::time::{Duration, Instant};

/// 用调用方同一期限和检查点推进固定启动握手，不把Ready/EOF当执行或发布许可。
/// 来源：原生 Rust nonblocking Init/Ready/ACK 与 pidfd ownership 复检。
pub(super) struct LinuxAtomicHandshake;

impl LinuxAtomicHandshake {
    /// 参数：child 是原子owner、deadline为原期限、checkpoint借原授权；返回：渠道终止或原错误。
    pub(super) fn run<E>(
        child: &mut LinuxAtomicChild,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), ChildSpawnError<E>> {
        Self::send(child, 3, deadline, checkpoint)?;
        loop {
            Self::check(deadline, checkpoint)?;
            match Self::receive(child.startup_fd()?, deadline, checkpoint)? {
                None => std::thread::sleep(Duration::from_millis(1)),
                Some(None) => {
                    return Err(ChildError::io(
                        "atomic startup before Ready",
                        io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "child closed startup before Ready",
                        ),
                    )
                    .into());
                }
                Some(Some(message)) => {
                    message.verify()?;
                    break;
                }
            }
        }
        Self::send(child, 4, deadline, checkpoint)?;
        loop {
            Self::check(deadline, checkpoint)?;
            match Self::receive(child.startup_fd()?, deadline, checkpoint)? {
                None => std::thread::sleep(Duration::from_millis(1)),
                Some(None) => {
                    // 原关系丢失仍失败；正常快速退出保真实 exit，但 EOF 自身从不设置 exec-success。
                    child.poll()?;
                    child.finish_startup();
                    Self::check(deadline, checkpoint)?;
                    return Ok(());
                }
                Some(Some(message)) => {
                    message.verify()?;
                    return Err(ChildError::io(
                        "atomic startup after ACK",
                        io::Error::new(io::ErrorKind::InvalidData, "repeated Ready"),
                    )
                    .into());
                }
            }
        }
    }

    /// 参数：fd 为原启动 socket，deadline/checkpoint 沿用原请求；返回：有界消息或原错误。
    pub(super) fn receive<E>(
        fd: i32,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Option<Option<LinuxAtomicMessage>>, ChildSpawnError<E>> {
        Self::check(deadline, checkpoint)?;
        LinuxAtomicMessage::receive(fd).map_err(Into::into)
    }

    fn send<E>(
        child: &mut LinuxAtomicChild,
        kind: u32,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), ChildSpawnError<E>> {
        loop {
            Self::check(deadline, checkpoint)?;
            child.poll()?;
            let fd = child.startup_fd()?;
            match LinuxAtomicMessage::send(fd, kind) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(primary) => {
                    // 子初始化可在Init之前实际失败并退出。仅领取同通道已排队的
                    // 固定Error；EOF/Pending/Ready绝不改写为发送或执行成功。
                    if let Ok(Some(Some(message))) = LinuxAtomicMessage::receive(fd)
                        && message.kind == 2
                        && let Err(error) = message.verify()
                    {
                        return Err(error.into());
                    }
                    return Err(primary.into());
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn check<E>(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        if Instant::now() >= deadline {
            return Err(ChildError::io(
                "atomic startup original deadline",
                io::Error::new(io::ErrorKind::TimedOut, "original startup deadline elapsed"),
            )
            .into());
        }
        Ok(())
    }
}
