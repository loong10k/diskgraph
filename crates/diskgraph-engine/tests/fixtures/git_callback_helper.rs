//! 仅由临时 Git 回归编译的原生子程序；不作为 shipping CLI 或安装工具。

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    if std::env::current_exe()
        .unwrap()
        .file_stem()
        .is_some_and(|name| name == "git")
    {
        // 同名伪工具只记录本临时目录的实际执行，不接触仓库源元数据或网络。
        let marker = std::env::current_exe().unwrap().with_extension("marker");
        assert_owned_sibling(&marker);
        std::fs::write(marker, b"shadow-executed").unwrap();
        println!("git version 999.0.0-test-shadow");
        return;
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--no-pager") {
        git_shim(&args);
        return;
    }
    let mode = args
        .first()
        .and_then(|arg| arg.to_str())
        .expect("fixed mode");
    let marker = Path::new(args.get(1).expect("temporary marker"));
    assert!(marker.is_absolute() && marker.parent().unwrap().is_dir());
    assert_owned_sibling(marker);
    std::fs::write(marker, mode.as_bytes()).unwrap();
    match mode {
        "fsmonitor" => std::io::stdout().write_all(b"fixture-token\0/\0").unwrap(),
        "clean" => {
            let mut input = Vec::new();
            std::io::stdin()
                .lock()
                .take(65_537)
                .read_to_end(&mut input)
                .unwrap();
            assert!(input.len() <= 65_536, "fixture input limit");
            std::io::stdout()
                .write_all(&input.to_ascii_uppercase())
                .unwrap();
        }
        // 真实启动后故意拒绝长进程协议，正控制须观察 marker 和 Git 的失败退出。
        "process" => std::process::exit(47),
        _ => panic!("unknown fixed callback mode"),
    }
}

fn git_shim(args: &[std::ffi::OsString]) {
    let settings = std::env::current_exe().unwrap().with_extension("settings");
    let data = std::fs::read_to_string(settings).unwrap();
    let fields: Vec<_> = data.lines().collect();
    assert_eq!(fields.len(), 3);
    assert!(Path::new(fields[0]).is_absolute());
    assert_owned_sibling(Path::new(fields[1]));
    assert_owned_sibling(Path::new(fields[2]));
    let status = Command::new(fields[0])
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap();
    if status.success() && args.iter().any(|arg| arg == "status") {
        let path = PathBuf::from(std::env::var_os("GIT_INDEX_FILE").unwrap());
        assert_eq!(path.file_name().unwrap(), "index");
        let repo = path.parent().unwrap();
        assert_eq!(repo.file_name().unwrap(), "repo");
        assert!(
            repo.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("diskgraph-git-")
        );
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let before = observe(&file);
        file.seek(SeekFrom::Start(7)).unwrap();
        let mut byte = [0];
        file.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Start(7)).unwrap();
        file.write_all(&[byte[0] ^ 255]).unwrap();
        file.sync_all().unwrap();
        // Windows 写句柄关闭后才能读取最终修改水位；观察者不持有原写句柄。
        drop(file);
        let after = observe(&File::open(&path).unwrap());
        let metrics = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            before.0,
            before.1,
            after.0,
            after.1,
            before.2,
            after.2,
            byte[0],
            byte[0] ^ 255
        );
        std::fs::write(fields[1], metrics).unwrap();
        std::fs::write(fields[2], path.to_str().unwrap()).unwrap();
    }
    std::process::exit(status.code().expect("real Git normal status"));
}

fn assert_owned_sibling(path: &Path) {
    let program = std::env::current_exe().unwrap();
    // 所有 marker/metrics 只允许位于本测试临时编译程序的同级目录。
    assert_eq!(
        path.parent().unwrap().canonicalize().unwrap(),
        program.parent().unwrap().canonicalize().unwrap()
    );
}

#[cfg(unix)]
fn observe(file: &File) -> (u64, u64, String) {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().unwrap();
    (
        metadata.len(),
        metadata.blocks().checked_mul(512).unwrap(),
        format!("{}:{}", metadata.dev(), metadata.ino()),
    )
}

#[cfg(windows)]
fn observe(file: &File) -> (u64, u64, String) {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    // 来源：WinBase FILE_INFO_BY_HANDLE_CLASS，FileStandardInfo=1、FileIdInfo=18。
    // 两个原生记录都是 24 字节，u64 数组提供对应 LARGE_INTEGER/volume 字段的对齐。
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetFileInformationByHandleEx"]
        fn file_information_by_handle_ex(
            handle: *mut c_void,
            class: i32,
            info: *mut c_void,
            bytes: u32,
        ) -> i32;
    }
    let mut standard = [0_u64; 3];
    let mut id = [0_u64; 3];
    assert_ne!(
        unsafe {
            file_information_by_handle_ex(file.as_raw_handle(), 1, standard.as_mut_ptr().cast(), 24)
        },
        0
    );
    assert_ne!(
        unsafe {
            file_information_by_handle_ex(file.as_raw_handle(), 18, id.as_mut_ptr().cast(), 24)
        },
        0
    );
    assert!(standard[0] <= i64::MAX as u64 && standard[1] <= i64::MAX as u64);
    (
        standard[1],
        standard[0],
        format!("{:016x}:{:016x}:{:016x}", id[0], id[1], id[2]),
    )
}
