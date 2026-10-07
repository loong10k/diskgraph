//! HTTP 发送阶段共用绝对期限的 socket 写入。
use std::io::{ErrorKind, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// 参数：原阻塞socket、顺序发送的切片、原绝对deadline和每次发送前的授权检查。
/// 返回：全部发送成功为()；原IO错误、期限耗尽或授权拒绝向上传播。
/// 临时使用非阻塞写，结束时恢复阻塞模式，使原请求reader可继续读取下一请求。
pub(crate) fn write_until(
    stream: &mut TcpStream,
    parts: &[&[u8]],
    deadline: Instant,
    mut authorize: impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    stream.set_nonblocking(true)?;
    let result = (|| {
        for part in parts {
            let mut remaining = *part;
            while !remaining.is_empty() {
                check_deadline(deadline)?;
                authorize()?;
                // 授权检查中的锁与SQL也消耗原窗口；返回后不能续期或继续过期发送。
                check_deadline(deadline)?;
                match stream.write(&remaining[..remaining.len().min(64 * 1024)]) {
                    Ok(0) => {
                        return Err(std::io::Error::new(
                            ErrorKind::WriteZero,
                            "HTTP delivery closed",
                        ));
                    }
                    Ok(count) => remaining = &remaining[count..],
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error)
                        if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                    {
                        std::thread::sleep(
                            deadline
                                .saturating_duration_since(Instant::now())
                                .min(Duration::from_millis(10)),
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        check_deadline(deadline)?;
        stream.flush()
    })();
    // 无论发送失败与否都恢复模式；原发送错误优先，成功时传播恢复错误。
    let restored = stream.set_nonblocking(false);
    result.and(restored)
}

fn check_deadline(deadline: Instant) -> std::io::Result<()> {
    if Instant::now() >= deadline {
        Err(std::io::Error::new(
            ErrorKind::TimedOut,
            "HTTP delivery deadline exceeded",
        ))
    } else {
        Ok(())
    }
}
