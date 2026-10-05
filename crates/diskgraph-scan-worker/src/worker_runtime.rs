use crate::{
    FrameReader, ProtocolLimits, worker_control::WorkerControl, worker_failure::WorkerFailure,
    worker_output::WorkerOutput, worker_request::WorkerRequest, worker_tree::WorkerTree,
};
use diskgraph_disktree_core::scan::{ScanHandle, ScanOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::time::Duration;

const INPUT_LIMITS: ProtocolLimits = ProtocolLimits {
    max_frame_bytes: 1024 * 1024,
    max_stream_bytes: 2 * 1024 * 1024,
    max_nodes: 1,
    max_depth: 0,
};

/// 真实 helper 标准管道入口；来源：PF-06 与 pinned ScanHandle，专用于独立进程 main。
/// 参数：无显式参数；只读取继承 stdin，不读取 PATH、数据库、授权或远程 executable 参数。
/// 返回：不返回；控制线程真实 join 后以 OS 退出结束本 helper，失败非零。
/// 原 pinned channel/线程可能仍持有 Node，进程退出回收它们，避免失败路径递归 Drop。
pub fn run_worker_stdio() -> ! {
    let mut reader = FrameReader::new(io::stdin(), INPUT_LIMITS);
    let prepared = prepare(&mut reader);
    let (root, options, limits) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            let mut output = WorkerOutput::new(io::stdout().lock(), INPUT_LIMITS);
            if output
                .failure(&WorkerFailure::new("protocol", error))
                .is_err()
            {
                diagnostic();
            }
            process::exit(1);
        }
    };
    let mut output = WorkerOutput::new(io::stdout().lock(), limits);
    if output.hello().is_err() {
        diagnostic();
        process::exit(1);
    }
    let scan = ScanHandle::spawn(root, options);
    let mut control = match WorkerControl::start(reader, Arc::clone(&scan.progress)) {
        Ok(control) => control,
        Err(error) => {
            let _ = output.failure(&WorkerFailure::new("control", error));
            process::exit(1);
        }
    };
    let result = scan_to_output(&scan, &control, &mut output);
    let successful = result.is_ok();
    if let Err(error) = result {
        scan.cancel();
        control.mark_terminal();
        if output.failure(&error).is_err() {
            diagnostic();
        }
    }
    // End/Error 后父关闭 stdin；部分输入或不合作的父端可能阻塞，交父 Child owner 终止回收。
    let joined = control.finish();
    if successful && joined.is_ok() && control.check().is_ok() {
        process::exit(0)
    } else {
        process::exit(1)
    }
}

fn prepare(
    reader: &mut FrameReader<io::Stdin>,
) -> io::Result<(PathBuf, ScanOptions, ProtocolLimits)> {
    match reader.read_payload::<WorkerRequest>()? {
        Some(request) => request.into_scan(),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected one execution Request version 2",
        )),
    }
}

fn scan_to_output<W: Write>(
    scan: &ScanHandle,
    control: &WorkerControl,
    output: &mut WorkerOutput<W>,
) -> Result<(), WorkerFailure> {
    loop {
        control.check()?;
        if let Some(result) = scan.poll() {
            // 先接管原树，即使末段取消/输出失败也不递归销毁深树。
            let tree = result
                .map(WorkerTree::new)
                .map_err(|error| WorkerFailure::new("scan_io", error))?;
            control.check()?;
            output
                .progress(scan.progress.snapshot())
                .map_err(|error| WorkerFailure::new("output", error))?;
            return output.tree(tree.root(), control);
        }
        output
            .progress(scan.progress.snapshot())
            .map_err(|error| WorkerFailure::new("output", error))?;
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn diagnostic() {
    // 只有固定有界说明可写 stderr；真实错误走有界 stdout Error，不打印路径/任意上游字符串。
    let _ = io::stderr()
        .lock()
        .write_all(b"diskgraph scan worker: output failed\n");
}
