//! 恢复执行后的真实DLL加载正负控制；不以挂起策略位替代加载器验收。
use super::WindowsChild;
use super::windows_control_fixture::command;
use crate::native_child::{ChildInputMode, ControlWriteStatus};
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemDirectoryW(buffer: *mut u16, length: u32) -> u32;
    fn LoadLibraryW(path: *const u16) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

fn load(path: &Path, marker: bool) -> (bool, Option<i32>) {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    if module.is_null() {
        return (false, std::io::Error::last_os_error().raw_os_error());
    }
    if marker {
        assert!(
            !unsafe { GetProcAddress(module, c"qualification_marker".as_ptr().cast()) }.is_null()
        );
    }
    assert_ne!(unsafe { FreeLibrary(module) }, 0);
    (true, None)
}

/// 参数：directory为该真实子进程的独占夹具目录；返回：写入实际加载结果。
/// 从恢复后的子进程调用Windows加载器，既不修改政策，也不把错误模拟成拒绝。
pub(super) fn run_child(directory: &Path) {
    let mut wide = vec![0_u16; 32768];
    let count = unsafe { GetSystemDirectoryW(wide.as_mut_ptr(), wide.len() as u32) } as usize;
    assert!(count > 0 && count < wide.len());
    use std::os::windows::ffi::OsStringExt;
    let system = std::path::PathBuf::from(std::ffi::OsString::from_wide(&wide[..count]));
    let signed = load(&system.join("version.dll"), false);
    let unsigned = load(&directory.join("unsigned.dll"), true);
    std::fs::write(
        directory.join("loader-result.json"),
        serde_json::to_vec(&serde_json::json!({
            "system32_loaded": signed.0,
            "system32_error": signed.1,
            "unsigned_loaded": unsigned.0,
            "unsigned_error": unsigned.1,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn build_unsigned(directory: &Path) -> [u8; 32] {
    // 全新本地C源及实际MSVC链接，不使用预制PE字节或错误文件冒充可加载DLL。
    std::fs::write(
        directory.join("unsigned.c"),
        "__declspec(dllexport) int qualification_marker(void) { return 7; }\n\
         int __stdcall DllMain(void *image, unsigned long reason, void *reserved) {\n\
         (void)image; (void)reason; (void)reserved; return 1; }\n",
    )
    .unwrap();
    let target = env!("DISKGRAPH_ENGINE_TARGET");
    let tool = cc::Build::new()
        .target(target)
        .host(target)
        .opt_level(0)
        .debug(false)
        .cargo_metadata(false)
        .get_compiler();
    assert!(tool.is_like_msvc(), "native MSVC qualification is required");
    let result = tool
        .to_command()
        .current_dir(directory)
        .args([
            "/LD",
            "/GS-",
            "/Zl",
            "unsigned.c",
            "/Fe:unsigned.dll",
            "/link",
            "/NODEFAULTLIB",
            "/ENTRY:DllMain",
            "/IMPLIB:unsigned.lib",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "actual DLL build failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Sha256::digest(std::fs::read(directory.join("unsigned.dll")).unwrap()).into()
}

fn observed(mode: ChildInputMode) -> serde_json::Value {
    let directory = tempfile::tempdir().unwrap();
    let digest = build_unsigned(directory.path());
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut owner = None;
    let birth = WindowsChild::spawn_into_with_admission(
        &mut command("dll-policy", directory.path()),
        mode,
        &mut owner,
        || check(deadline),
        || check(deadline),
    );
    let result = {
        birth.unwrap();
        let child = owner.as_mut().expect("original resumed child owner");
        let job = child.duplicate_job_for_test().unwrap();
        let mut closed = mode == ChildInputMode::Null;
        let mut close_polling = false;
        let mut output_bytes = 0usize;
        loop {
            check(deadline).unwrap();
            if !closed {
                let status = if close_polling {
                    child.poll_control_write().unwrap()
                } else {
                    child.request_control_close().unwrap()
                };
                match status {
                    ControlWriteStatus::Closed => closed = true,
                    ControlWriteStatus::Pending => close_polling = true,
                    ControlWriteStatus::Written(_) => {
                        panic!("unexpected write during empty control close")
                    }
                }
            }
            if let Some(bytes) = child.read_stdout().unwrap() {
                output_bytes += bytes.len();
            }
            if let Some(bytes) = child.read_stderr().unwrap() {
                output_bytes += bytes.len();
            }
            assert!(output_bytes <= 65536, "fixture output budget");
            if child.stdout_eof() && child.stderr_eof() && child.poll().unwrap() {
                use windows_sys::Win32::System::JobObjects::{
                    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
                    QueryInformationJobObject,
                };
                let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                assert_ne!(
                    unsafe {
                        QueryInformationJobObject(
                            job.as_raw(),
                            JobObjectBasicAccountingInformation,
                            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                            std::mem::size_of_val(&accounting) as u32,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                if accounting.ActiveProcesses == 0
                    && (mode == ChildInputMode::Null
                        || child.poll_normal_exit(&mut || check(deadline)).unwrap())
                {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            child.exit_code(),
            Some(0),
            "loader child did not exit naturally"
        );
        serde_json::from_slice(&std::fs::read(directory.path().join("loader-result.json")).unwrap())
            .unwrap()
    };
    // 断言/解析失败也由块外的原owner Drop保底，成功必须先有真实whole-job退出。
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(
            std::fs::read(directory.path().join("unsigned.dll")).unwrap()
        )),
        digest
    );
    result
}

fn check(deadline: Instant) -> Result<(), std::io::Error> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
    }
}

#[test]
fn resumed_scan_rejects_unsigned_dll_and_loads_system32_dependency() {
    let result = observed(ChildInputMode::WorkerControl);
    assert_eq!(result["system32_loaded"], true, "{result}");
    assert_eq!(result["unsigned_loaded"], false, "{result}");
    assert_eq!(
        result["unsigned_error"], 577,
        "expected actual invalid image hash: {result}"
    );
    eprintln!("DG_SCAN_UNSIGNED_REJECTED_SYSTEM32_LOADED={result}");
}

#[test]
fn resumed_null_probe_loads_the_same_valid_unsigned_dll() {
    let result = observed(ChildInputMode::Null);
    assert_eq!(result["system32_loaded"], true, "{result}");
    assert_eq!(result["unsigned_loaded"], true, "{result}");
    eprintln!("DG_NULL_UNSIGNED_DLL_POSITIVE={result}");
}
