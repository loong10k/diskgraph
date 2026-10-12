use super::linux_broker::send;
use crate::LinuxSupervisorProcess;
use crate::recovery_control::{
    ControlNotification, FrontendRead, LinuxControlRead, LinuxFrontendConnection,
};
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// 参数：原executor、原客户连接和原停止信号；返回：完整MCP前台终态或原交付错误。
/// 长连接不延长任何MCP请求预算；单笔输入/输出30秒，原资源退休始终独立。
pub(super) fn deliver(
    owner: &mut LinuxSupervisorProcess,
    connection: &mut LinuxFrontendConnection,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut exit = None;
    let mut input_ended = false;
    let mut total = 0u64;
    loop {
        let deadline = Instant::now() + Duration::from_secs(30);
        if !input_ended && exit.is_none() {
            match connection
                .poll_input_until(deadline, cancel)
                .map_err(io::Error::other)?
            {
                FrontendRead::Data(bytes) => {
                    charge(&mut total, bytes.len())?;
                    owner.write_stdin_until(bytes, deadline, cancel)?;
                }
                FrontendRead::Eof => {
                    owner.finish_stdin()?;
                    input_ended = true;
                }
                FrontendRead::Pending => {}
            }
        }
        match owner.poll_notification(cancel)? {
            LinuxControlRead::Frame(frame) => {
                if let ControlNotification::ForegroundEnded { exit_code, .. } = frame.notification {
                    exit = Some(exit_code);
                }
            }
            LinuxControlRead::ConfirmedEof if exit.is_none() => {
                return Err(io::Error::other("missing original MCP foreground result"));
            }
            _ => {}
        }
        if let Some(bytes) = owner.read_stdout()? {
            charge(&mut total, bytes.len())?;
            send(connection, 1, bytes, deadline, cancel)?;
        }
        if let Some(bytes) = owner.read_stderr()? {
            charge(&mut total, bytes.len())?;
            send(connection, 2, bytes, deadline, cancel)?;
        }
        if let Some(code) = exit
            && owner.foreground_output_drained()?
        {
            send(connection, 3, &code.to_be_bytes(), deadline, cancel)?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn charge(total: &mut u64, bytes: usize) -> io::Result<()> {
    *total = total
        .checked_add(bytes as u64)
        .ok_or_else(|| io::Error::other("MCP session byte overflow"))?;
    if *total > 2 << 30 {
        return Err(io::Error::other("MCP session stream budget"));
    }
    Ok(())
}
