use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetTickCount64"]
    fn get_tick_count64() -> u64;
}

/// 测试独占的原生探针 owner；只验证组件事实，不授予受信镜像执行许可。来源：PF06 Windows 镜像租约探针。
pub(super) struct WindowsImageLeaseProbe {
    root: TempDir,
    child: Option<Child>,
    stdout: Option<JoinHandle<(Vec<u8>, Option<String>, bool)>>,
    stderr: Option<JoinHandle<(Vec<u8>, Option<String>, bool)>>,
    started: Instant,
    deadline: Instant,
    hashes: Value,
}

impl WindowsImageLeaseProbe {
    /// 单次运行真实 API 探针并保留完整固定标量见证。参数：case_id 为一至七；返回：实际退出状态和有界 JSON，不重试。
    pub(super) fn run(case_id: u32) -> (ExitStatus, Value) {
        let started = Instant::now();
        let deadline = started + Duration::from_secs(20);
        // Win32 同一系统单调计数跨进程传绝对期限，不在 C 子进程刷新准备时间。
        let native_deadline = unsafe { get_tick_count64() }
            .checked_add(20_000)
            .expect("native original deadline must be representable");
        let executable = artifact("DISKGRAPH_WINDOWS_IMAGE_LEASE_PROBE");
        let image_a = artifact("DISKGRAPH_WINDOWS_IMAGE_A");
        let image_b = artifact("DISKGRAPH_WINDOWS_IMAGE_B");
        let hashes = json!({
            "probe_sha256": hash(&executable, deadline),
            "image_a_sha256": hash(&image_a, deadline),
            "image_b_sha256": hash(&image_b, deadline),
        });
        assert_ne!(hashes["image_a_sha256"], hashes["image_b_sha256"]);
        let root = TempDir::new().expect("owned Windows probe directory must be available");
        assert!(
            Instant::now() < deadline,
            "artifact preparation exhausted original 20s qualification"
        );
        let child = Command::new(executable)
            .arg(case_id.to_string())
            .arg(root.path())
            .arg(image_a)
            .arg(image_b)
            .arg(native_deadline.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("explicit native probe artifact must actually start");
        let mut owner = Self {
            root,
            child: Some(child),
            stdout: None,
            stderr: None,
            started,
            deadline,
            hashes,
        };
        // reader 创建失败仍由已构造的 owner 清理原 child；不把 thread spawn 当无失败步骤。
        // C fixture 的受测 children 仅继承私有 marker 输出 HANDLE，不继承本管道。
        let child = owner.child.as_mut().expect("original probe child retained");
        let stdout = child.stdout.take().expect("probe stdout must exist");
        let stderr = child.stderr.take().expect("probe stderr must exist");
        owner.stdout = Some(
            thread::Builder::new()
                .spawn(move || drain(stdout, 16 * 1024))
                .expect("stdout reader must start"),
        );
        owner.stderr = Some(
            thread::Builder::new()
                .spawn(move || drain(stderr, 4 * 1024))
                .expect("stderr reader must start"),
        );
        owner.finish(case_id)
    }

    fn finish(&mut self, case_id: u32) -> (ExitStatus, Value) {
        let mut timed_out = false;
        let mut polling_error = None;
        let status = loop {
            let child = self
                .child
                .as_mut()
                .expect("probe owner is retained until actual wait");
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    polling_error = Some(error.to_string());
                    let _ = child.kill();
                    break child
                        .wait()
                        .expect("rescue must wait for original probe child");
                }
            }
            if Instant::now() >= self.deadline {
                timed_out = true;
                let _ = child.kill();
                break child
                    .wait()
                    .expect("timeout rescue must wait for original probe child");
            }
            thread::sleep(Duration::from_millis(1));
        };
        self.child.take();
        let stdout = self.stdout.take().expect("stdout drain retained").join();
        let stderr = self.stderr.take().expect("stderr drain retained").join();
        let elapsed = self.started.elapsed();
        // 所有实际进程与 reader 先收尾，再作资格断言；救援不会充作成功。
        let (stdout, stdout_error, stdout_overflow) = stdout.expect("stdout reader must not panic");
        let (stderr, stderr_error, stderr_overflow) = stderr.expect("stderr reader must not panic");
        let text = String::from_utf8_lossy(&stdout);
        let parsed: Result<Value, _> = serde_json::from_slice(&stdout);
        eprintln!(
            "windows_image_lease case={case_id} status={status} elapsed_us={} timeout={timed_out} polling_error={polling_error:?} stdout={text} stderr={} artifact_hashes={}",
            elapsed.as_micros(),
            String::from_utf8_lossy(&stderr),
            self.hashes
        );
        assert!(
            !timed_out,
            "native probe timed out; rescue is not acceptance"
        );
        assert!(polling_error.is_none(), "native probe polling failed");
        assert!(
            stdout_error.is_none() && stderr_error.is_none(),
            "native output read failed"
        );
        assert!(
            !stdout_overflow && !stderr_overflow,
            "fixed native output limit exceeded"
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "original whole-call deadline exceeded"
        );
        assert!(stderr.is_empty(), "unexpected native fixture stderr");
        let mut record = parsed.expect("native probe must produce one complete JSON record");
        record["artifact_hashes"] = self.hashes.clone();
        record["whole_call_elapsed_us"] = json!(elapsed.as_micros());
        record["security_acceptance"] = json!(false);
        assert!(
            self.root.path().exists(),
            "owned fixture remains until cleanup and assertions"
        );
        (status, record)
    }
}

impl Drop for WindowsImageLeaseProbe {
    /// 异常路径清理测试自有探针及 reader。参数：无；返回：无，救援不作为原生验收。
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // 仅异常清理；C probe 的 Job close 杀掉它拥有的 marker 子进程。
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.stdout.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr.take() {
            let _ = reader.join();
        }
    }
}

fn artifact(name: &str) -> PathBuf {
    let value =
        std::env::var_os(name).unwrap_or_else(|| panic!("explicit {name} artifact is required"));
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "artifact path must be absolute");
    let metadata = path
        .symlink_metadata()
        .expect("explicit artifact metadata must exist");
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "artifact must be an actual ordinary file"
    );
    assert!(
        metadata.len() > 0 && metadata.len() <= 128 * 1024 * 1024,
        "artifact size qualification failed"
    );
    path
}

fn hash(path: &Path, deadline: Instant) -> String {
    let mut file = File::open(path).expect("qualified artifact must open");
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        assert!(
            Instant::now() < deadline,
            "artifact digest exhausted original deadline"
        );
        let bytes = file
            .read(&mut buffer)
            .expect("qualified artifact digest read must succeed");
        if bytes == 0 {
            break;
        }
        hash.update(&buffer[..bytes]);
    }
    format!("{:x}", hash.finalize())
}

fn drain(mut reader: impl Read, limit: usize) -> (Vec<u8>, Option<String>, bool) {
    let mut output = Vec::with_capacity(limit);
    let mut chunk = [0u8; 4096];
    let mut overflow = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return (output, None, overflow),
            Ok(bytes) => {
                let admitted = bytes.min(limit.saturating_sub(output.len()));
                output.extend_from_slice(&chunk[..admitted]);
                overflow |= admitted != bytes;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return (output, Some(error.to_string()), overflow),
        }
    }
}
