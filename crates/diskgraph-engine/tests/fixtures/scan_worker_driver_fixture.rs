//! 独立编译的父驱动阶段夹具，不挂 libtest；来源：真实 pinned ScanHandle 与执行 v2 codec。
//! 它不是受信安装 helper，也不证明扫描中取消；只有父协议与真实进程退场测试使用。
use diskgraph_disktree_core::scan::ScanHandle;
use diskgraph_scan_worker::{
    ExecutionFrame, FlatNodes, FrameReader, FrameWriter, ProtocolLimits, WorkerIoKind,
    WorkerRequest,
};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const TARGET: &str = "driver-fixture-target";
const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 65536,
        max_stream_bytes: 1048576,
        max_nodes: 4096,
        max_depth: 64,
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("driver fixture failed: {error}");
        std::process::exit(91);
    }
}

fn run() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().expect("fixed mode");
    let directory = std::path::PathBuf::from(args.next().expect("isolated directory"));
    // 只作为失败救援；成功测试不接受退出码 88。
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        std::process::exit(88);
    });
    std::fs::write(directory.join("pid"), std::process::id().to_string())?;
    if mode == "stderr-first" || mode == "stderr-limit" {
        // 大于各平台通常管道容量；父端必须公平排空，不能等待输出完成才开始读。
        for _ in 0..64 {
            io::stderr().write_all(&[b'd'; 4096])?;
        }
        io::stderr().flush()?;
        std::fs::write(directory.join("stderr-written"), b"true")?;
    }
    let mut stdin = io::stdin().lock();
    let mut reader = FrameReader::new(&mut stdin, limits());
    let request: WorkerRequest = reader
        .read_payload()?
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
    std::fs::write(
        directory.join("actual-request.json"),
        serde_json::to_vec(&request)?,
    )?;
    let (root, options, original_limits) = request.into_scan()?;
    std::fs::write(directory.join("request-complete"), b"true")?;
    let mut writer = FrameWriter::new(io::stdout().lock(), original_limits);
    let hello: ExecutionFrame = ExecutionFrame::Hello {
        version: 2,
        target: TARGET.into(),
        pin: PIN.into(),
    };
    writer.write_payload(&hello)?;
    writer.flush()?;
    if mode == "cancel-order" {
        let cancel: WorkerRequest = reader
            .read_payload()?
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        if !matches!(cancel, WorkerRequest::Cancel {}) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Cancel must follow complete Request",
            ));
        }
        std::fs::write(directory.join("cancel-after-request"), b"true")?;
        let failure: ExecutionFrame = ExecutionFrame::Error {
            code: "cancelled".into(),
            io_kind: WorkerIoKind::Interrupted,
            raw_os_error: None,
            message: "driver fixture received ordered Cancel".into(),
        };
        writer.write_payload(&failure)?;
        writer.flush()?;
        drop(writer);
        drop(reader);
        expect_eof(&mut stdin)?;
        std::fs::write(directory.join("control-eof"), b"true")?;
        std::process::exit(2);
    }
    if mode == "bad-frame" {
        drop(writer);
        io::stdout().write_all(&2u32.to_le_bytes())?;
        io::stdout().write_all(b"{]")?;
        io::stdout().flush()?;
        std::fs::write(directory.join("bad-frame-written"), b"true")?;
        hold(&directory)?;
        return Err(io::Error::other(
            "bad-frame fixture should have been reaped",
        ));
    }
    if mode == "checkpoint" {
        std::fs::write(directory.join("checkpoint-ready"), b"true")?;
        hold(&directory)?;
        return Err(io::Error::other(
            "checkpoint fixture should have been reaped",
        ));
    }
    let handle = ScanHandle::spawn(root, options);
    let start = Instant::now();
    let tree = loop {
        if let Some(result) = handle.poll() {
            break result?;
        }
        if start.elapsed() > Duration::from_secs(20) {
            handle.cancel();
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let mut count = 0;
    for node in FlatNodes::new(&tree) {
        writer.write_payload(&ExecutionFrame::<String, String>::Node { node: node? })?;
        count += 1;
    }
    writer.write_payload(&ExecutionFrame::<String, String>::End { nodes: count })?;
    writer.flush()?;
    drop(writer);
    drop(reader);
    expect_eof(&mut stdin)?;
    std::fs::write(directory.join("control-eof"), b"true")?;
    close_output()?;
    std::fs::write(directory.join("pipes-closed"), b"true")?;
    if mode == "held-end" {
        hold(&directory)?;
    }
    std::fs::write(directory.join("natural-exit"), b"true")?;
    std::process::exit(0);
}

fn expect_eof(input: &mut impl Read) -> io::Result<()> {
    let mut byte = [0];
    if input.read(&mut byte)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected control tail",
        ));
    }
    Ok(())
}

fn hold(directory: &Path) -> io::Result<()> {
    let mut sequence = 0u64;
    while !directory.join("release").exists() {
        sequence += 1;
        std::fs::write(directory.join("heartbeat"), sequence.to_le_bytes())?;
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

#[cfg(unix)]
fn close_output() -> io::Result<()> {
    unsafe extern "C" {
        fn close(fd: i32) -> i32;
    }
    for fd in [1, 2] {
        if unsafe { close(fd) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn close_output() -> io::Result<()> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(value: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    for value in [-11i32, -12i32] {
        if unsafe { CloseHandle(GetStdHandle(value as u32)) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
