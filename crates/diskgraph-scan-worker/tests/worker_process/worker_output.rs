use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const FRAME_BYTES: u64 = 1024 * 1024;
const STREAM_BYTES: u64 = 8 * 1024 * 1024;
const STDERR_BYTES: u64 = 64 * 1024;

/// 真实 helper 进程及有限管道读取的测试回执；来源：std::process/std::io，非 scanner 替身。
pub(crate) struct WorkerOutput {
    pub(crate) status: ExitStatus,
    pub(crate) frames: Vec<serde_json::Value>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_bytes: u64,
}

impl WorkerOutput {
    /// 参数：binary 为 Cargo 精确产物，directory 为隔离目录，packet 为唯一原请求。
    /// 返回：实际 wait 后的有界 stdout 帧与 stderr；超时先 kill/wait/join 后才断言失败。
    pub(crate) fn run(binary: &Path, directory: &Path, packet: &[u8]) -> Self {
        let mut child = Command::new(binary)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fresh Cargo worker binary must start");
        let mut input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let write = input.write_all(packet);
        let reader = std::thread::spawn(move || {
            let mut input = Some(input);
            let mut source = stdout;
            let mut frames = Vec::new();
            let mut total = 0_u64;
            loop {
                let mut header = [0_u8; 4];
                match source.read(&mut header[..1]) {
                    Ok(0) => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                    Ok(_) => {}
                }
                source.read_exact(&mut header[1..])?;
                let length = u64::from(u32::from_le_bytes(header));
                total = total
                    .checked_add(4)
                    .and_then(|n| n.checked_add(length))
                    .filter(|n| *n <= STREAM_BYTES)
                    .ok_or_else(|| io::Error::other("helper stdout stream exceeded"))?;
                if length > FRAME_BYTES {
                    return Err(io::Error::other("helper stdout frame exceeded"));
                }
                let mut body = vec![0; usize::try_from(length).unwrap()];
                source.read_exact(&mut body)?;
                let frame: serde_json::Value =
                    serde_json::from_slice(&body).map_err(io::Error::other)?;
                if matches!(
                    frame.get("type").and_then(serde_json::Value::as_str),
                    Some("end" | "error")
                ) {
                    // 终态后先关闭控制写端，helper 才能 join 控制 reader 并关闭 stdout。
                    drop(input.take());
                }
                frames.push(frame);
            }
            Ok::<_, io::Error>((frames, total))
        });
        let diagnostics = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take(STDERR_BYTES + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait().expect("timed-out child must be reaped");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let read = reader.join();
        let diagnostics = diagnostics.join();
        assert!(
            !timed_out,
            "helper failed to reach EOF and actual exit in 30s: {status:?}"
        );
        write.expect("request write must reach actual helper stdin");
        let stderr = diagnostics
            .expect("stderr reader panic")
            .expect("stderr read failure");
        assert!(
            stderr.len() as u64 <= STDERR_BYTES,
            "stderr exceeded independent bound"
        );
        let (frames, stdout_bytes) = read
            .expect("stdout reader panic")
            .expect("invalid bounded stdout");
        Self {
            status,
            frames,
            stderr,
            stdout_bytes,
        }
    }
}
