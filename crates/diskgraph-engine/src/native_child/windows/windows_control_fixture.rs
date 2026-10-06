//! 独立 Windows 子进程测试夹具；真实读取继承 stdin 并以文件门控制接收阶段。

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::SetEvent;

/// 配置本测试二进制的真实子入口。参数：mode 为夹具行为，directory 为本请求自有目录。返回：绝对 executable 与显式环境命令，不查 PATH。
pub(super) fn command(mode: &str, directory: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "native_child::windows::windows_control_fixture::control_child_fixture",
            "--nocapture",
            "--quiet",
        ])
        .env_clear()
        .env("DG_CONTROL_CASE", mode)
        .env("DG_CONTROL_DIR", directory);
    command
}

#[test]
fn control_child_fixture() {
    let Ok(mode) = std::env::var("DG_CONTROL_CASE") else {
        return;
    };
    let directory = std::path::PathBuf::from(std::env::var_os("DG_CONTROL_DIR").unwrap());
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        std::process::exit(88);
    });
    if mode == "dll-policy" {
        super::windows_dll_policy_tests::run_child(&directory);
        std::process::exit(0);
    }
    if mode == "stamp" {
        std::fs::write(directory.join("started"), b"started").unwrap();
    }
    if mode == "inherit" {
        for standard in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            assert_eq!(
                unsafe { GetFileType(GetStdHandle(standard)) },
                FILE_TYPE_PIPE
            );
        }
        let raw: usize = std::env::var("DG_CONTROL_UNLISTED_EVENT")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { SetEvent(raw as HANDLE) },
            0,
            "unlisted parent event leaked into child"
        );
        while !directory.join("parent-event").exists() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let raw: usize = std::fs::read_to_string(directory.join("parent-event"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { SetEvent(raw as HANDLE) },
            0,
            "control parent event leaked into child"
        );
    }
    std::io::stdout().write_all(b"control-stdout\n").unwrap();
    std::io::stderr().write_all(b"control-stderr\n").unwrap();
    std::fs::write(directory.join("ready"), b"ready").unwrap();
    if mode == "hold" {
        while !directory.join("release").exists() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut received = std::fs::File::create(directory.join("received")).unwrap();
    let mut stdin = std::io::stdin().lock();
    let mut block = [0; 4096];
    loop {
        let n = stdin.read(&mut block).unwrap();
        if n == 0 {
            break;
        }
        received.write_all(&block[..n]).unwrap();
    }
    std::fs::write(directory.join("eof"), b"eof").unwrap();
    std::process::exit(0);
}
