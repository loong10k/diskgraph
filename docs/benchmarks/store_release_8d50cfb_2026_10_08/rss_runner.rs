// Linux 隔离测量：父进程实际 wait 后读取唯一测试子进程的 rusage，不包含编译器。
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(!args.is_empty());
    let status = std::process::Command::new(&args[0]).args(&args[1..]).status().unwrap();
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) }, 0);
    let usage = unsafe { usage.assume_init() };
    let bytes = u64::try_from(usage.ru_maxrss).unwrap().checked_mul(1024).unwrap();
    println!("DG_PROCESS_RUSAGE max_rss_bytes={bytes}");
    std::process::exit(status.code().unwrap_or(101));
}
