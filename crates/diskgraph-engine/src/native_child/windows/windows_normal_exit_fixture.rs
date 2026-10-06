//! Windows B 的真实子入口；End 字节、EOF、leader 与普通 Job 后代分别见证。

use super::windows_normal_process_witness::WindowsNormalProcessWitness;
use std::io::{self, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

/// 配置绝对测试二进制，不使用 PATH 或 shell。参数：mode 指定真实进程阶段，directory 为本请求目录。返回：显式环境命令。
pub(super) fn command(mode: &str, directory: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "native_child::windows::windows_normal_exit_fixture::normal_exit_child_fixture",
            "--nocapture",
            "--quiet",
        ])
        .env_clear()
        .env("DG_NORMAL_CASE", mode)
        .env("DG_NORMAL_DIR", directory);
    command
}

/// 提供已知 End 帧的原始传输字节，不证明完整协议树。参数：无。返回：4 字节长度及固定 JSON 正文。
pub(super) fn end_marker() -> Vec<u8> {
    let body = b"{\"type\":\"end\",\"nodes\":0}";
    let mut marker = (body.len() as u32).to_le_bytes().to_vec();
    marker.extend_from_slice(body);
    marker
}

fn heartbeat_until_release(directory: &Path, name: &str) -> io::Result<()> {
    let mut sequence = 0u64;
    while !directory.join("release").exists() {
        sequence += 1;
        std::fs::write(directory.join(name), sequence.to_le_bytes())?;
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn emit_end_and_close_standard_pipes() -> io::Result<()> {
    std::io::stdout().write_all(&end_marker())?;
    std::io::stdout().flush()?;
    std::io::stderr().write_all(b"normal-stderr\n")?;
    std::io::stderr().flush()?;
    for standard in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        if unsafe { CloseHandle(GetStdHandle(standard)) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[test]
fn normal_exit_child_fixture() {
    let Ok(mode) = std::env::var("DG_NORMAL_CASE") else {
        return;
    };
    let directory = std::path::PathBuf::from(std::env::var_os("DG_NORMAL_DIR").unwrap());
    // 仅救援：88 必须被父断言拒绝，不可作为自然退出正控。
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        std::process::exit(88);
    });
    if mode == "descendant" {
        heartbeat_until_release(&directory, "descendant-heartbeat").unwrap();
        std::fs::write(directory.join("descendant-natural"), b"natural").unwrap();
        std::process::exit(0);
    }
    if mode == "leader-with-descendant" {
        // std::process::Command 在 Windows 继承所有可继承句柄；先取消本 leader
        // 原 STD 句柄的继承标志，使普通后代确实仅持自己的 NUL，而非暗持原管道。
        for standard in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if unsafe { SetHandleInformation(GetStdHandle(standard), HANDLE_FLAG_INHERIT, 0) } == 0
            {
                panic!(
                    "clear fixture standard inheritance: {}",
                    io::Error::last_os_error()
                );
            }
        }
        let mut descendant = command("descendant", &directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // 从出生时持有的原Child句柄记录身份，父进程不能拿任意Job成员冒充后代。
        let identity = WindowsNormalProcessWitness::record(descendant.as_raw_handle()).unwrap();
        std::fs::write(directory.join("descendant-identity"), identity).unwrap();
        let started = Instant::now();
        while !directory.join("descendant-heartbeat").exists() {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        // 普通 CreateProcess 后代真实继承 Job；不使用 breakaway，也不终止它。
        // fixture leader 必须先退出，背景 wait 不是父正常完成的见证；Job 活动数另查。
        std::thread::spawn(move || {
            let _ = descendant.wait();
        });
    }
    emit_end_and_close_standard_pipes().unwrap();
    std::fs::write(directory.join("standard-pipes-closed"), b"closed").unwrap();
    match mode.as_str() {
        "live-leader" => {
            heartbeat_until_release(&directory, "leader-heartbeat").unwrap();
            std::fs::write(directory.join("leader-natural"), b"natural").unwrap();
            std::process::exit(0);
        }
        "leader-with-descendant" => std::process::exit(0),
        "nonzero-exit" => std::process::exit(7),
        _ => panic!("unexpected normal-exit fixture mode"),
    }
}
