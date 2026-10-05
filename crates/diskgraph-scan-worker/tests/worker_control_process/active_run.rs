use super::control_action::ControlAction;
use diskgraph_scan_worker::{FrameReader, ProtocolLimits};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// 真实 Cargo helper 运行与主动控制的回执；来源：std Child/管道，不证明 group 为空或 pinned 线程 join。
pub(crate) struct ActiveRun {
    pub(crate) status: ExitStatus,
    pub(crate) output: Value,
    pub(crate) stderr: Vec<u8>,
    pub(crate) timed_out: bool,
    pub(crate) elapsed: Duration,
}

impl ActiveRun {
    /// 参数：binary 为 Cargo 产物，directory 为独占 fixture，request 为有界原请求，action 为一次动作。
    /// 返回：实际 leader wait 且两 reader 已 join 的回执；救援 kill 不计为主动控制成功。
    pub(crate) fn run(
        binary: &Path,
        directory: &Path,
        request: Vec<u8>,
        action: ControlAction,
    ) -> io::Result<Self> {
        let started = Instant::now();
        let deadline = started + Duration::from_secs(30);
        let mut child = Command::new(binary)
            .current_dir(directory)
            // 只限制本测试 child 的实际 pinned walk pool；不改宿主或 helper 生产策略。
            .env("RAYON_NUM_THREADS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("configured stdin");
        let stdout = child.stdout.take().expect("configured stdout");
        let stderr = child.stderr.take().expect("configured stderr");
        let reader = match thread::Builder::new()
            .name("worker-control-stdout".into())
            .spawn(move || observe(stdout, stdin, request, action, started, deadline))
        {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let diagnostics = match thread::Builder::new()
            .name("worker-control-stderr".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                stderr.take(65_537).read_to_end(&mut bytes).map(|_| bytes)
            }) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(error);
            }
        };
        let mut timed_out = false;
        let waited = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(error);
                }
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait();
            }
            thread::sleep(Duration::from_millis(5));
        };
        // try_wait(Some) 已回收 leader；先完成双方 reader，即使结果将失败也不遗弃测试线程。
        let output = reader.join();
        let stderr = diagnostics.join();
        let status = waited?;
        let output = output.map_err(|_| io::Error::other("stdout observer panicked"))??;
        let stderr = stderr.map_err(|_| io::Error::other("stderr observer panicked"))??;
        Ok(Self {
            status,
            output,
            stderr,
            timed_out,
            elapsed: started.elapsed(),
        })
    }
}

fn observe(
    stdout: ChildStdout,
    stdin: ChildStdin,
    request: Vec<u8>,
    action: ControlAction,
    started: Instant,
    deadline: Instant,
) -> io::Result<Value> {
    let mut input = Some(stdin);
    // 写请求也在同一总时钟内，并由独立 supervisor 处理阻塞/异常，不在写完后重开计时。
    input.as_mut().expect("initial stdin").write_all(&request)?;
    let mut reader = FrameReader::new(
        stdout,
        ProtocolLimits {
            max_frame_bytes: 1_048_576,
            max_stream_bytes: 33_554_432,
            max_nodes: 40_000,
            max_depth: 4,
        },
    );
    let mut qualified = None;
    let mut action_elapsed_us = None;
    let mut action_before_deadline = false;
    let mut action_completed = false;
    let mut first = None;
    let mut error = None;
    let mut errors = 0_u64;
    let mut ends = 0_u64;
    let mut nodes = 0_u64;
    let mut frames = 0_u64;
    let mut progress = 0_u64;
    while let Some(frame) = reader.read_payload::<Value>()? {
        if first.is_none() {
            first = Some(frame.clone());
        }
        frames += 1;
        match frame["type"].as_str() {
            Some("progress") => {
                progress += 1;
                let snapshot = &frame["progress"];
                let active = snapshot["finished"] == false
                    && snapshot["cancelled"] == false
                    && (snapshot["files"].as_u64().unwrap_or(0) > 0
                        || snapshot["dirs"].as_u64().unwrap_or(0) > 0);
                if qualified.is_none() && active && ends == 0 && errors == 0 {
                    qualified = Some(frame.clone());
                    action_elapsed_us = Some(started.elapsed().as_micros());
                    action_before_deadline = Instant::now() < deadline;
                    if !action_before_deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "qualification arrived after original deadline",
                        ));
                    }
                    match action.payload() {
                        Some(body) => {
                            let stream = input
                                .as_mut()
                                .ok_or_else(|| io::Error::other("stdin closed before action"))?;
                            stream.write_all(&u32::try_from(body.len()).unwrap().to_le_bytes())?;
                            stream.write_all(body)?;
                            // Cancel 保持写端，直到真正 Error；否则 EOF 可能先覆盖取消分类。
                        }
                        None => {
                            drop(input.take());
                        }
                    }
                    action_completed = true;
                }
            }
            Some("node") => {
                nodes += 1;
            }
            Some("end") => {
                ends += 1;
                drop(input.take());
            }
            Some("error") => {
                errors += 1;
                error = Some(frame);
                drop(input.take());
            }
            Some("hello") if frames == 1 => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected runtime output frame",
                ));
            }
        }
    }
    Ok(
        json!({"first":first,"qualified":qualified,"action_elapsed_us":action_elapsed_us,
        "action_before_deadline":action_before_deadline,"action_completed":action_completed,
        "frames":frames,"progress_frames":progress,"node_frames":nodes,"ends":ends,"errors":errors,
        "error":error,"stdout_bytes":reader.bytes_read(),"clean_stdout_eof":true}),
    )
}
