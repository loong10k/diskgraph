//! 隔离 Linux 权限夹具的精确身份与文件访问观察；不属于产品身份授权入口。
unsafe extern "C" {
    fn geteuid() -> u32;
    fn getegid() -> u32;
    fn getgroups(size: i32, list: *mut u32) -> i32;
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(args.len(), 6);
    let expected_uid: u32 = args[1].to_str().unwrap().parse().unwrap();
    let expected_gid: u32 = args[2].to_str().unwrap().parse().unwrap();
    let expected_groups: Vec<u32> = match args[3].to_str().unwrap() {
        "none" => Vec::new(),
        group => vec![group.parse().unwrap()],
    };
    // 核对真正执行读取的进程身份；启动工具出错不能伪装成预期拒绝。
    assert_eq!(unsafe { geteuid() }, expected_uid);
    assert_eq!(unsafe { getegid() }, expected_gid);
    let count = unsafe { getgroups(0, std::ptr::null_mut()) };
    assert!((0..=16).contains(&count));
    let mut groups = [0_u32; 16];
    assert_eq!(unsafe { getgroups(16, groups.as_mut_ptr()) }, count);
    assert_eq!(&groups[..count as usize], expected_groups);
    let result = std::fs::read(&args[4]);
    match args[5].to_str().unwrap() {
        "allow" => assert_eq!(result.unwrap(), b"synthetic"),
        "deny" => assert_eq!(result.unwrap_err().raw_os_error(), Some(13)),
        _ => panic!("unknown fixture expectation"),
    }
}
