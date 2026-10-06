use std::ffi::CStr;

unsafe extern "C" {
    fn diskgraph_macos_native_spawn(
        path: *const libc::c_char,
        input: i32,
        output: i32,
        diagnostic: i32,
        pid: *mut libc::pid_t,
        owns_group: *mut bool,
    ) -> i32;
}

/// SDK编译C ABI的有界桥接，仅写入已存在的原owner字段。
/// 来源：原生 Rust PF-06 macOS native spawn 合同；无 Java 对等对象。
pub(super) struct MacosNativeSpawn;

impl MacosNativeSpawn {
    /// 出生并直接登记所有权。参数：为固定路径、三个持有子端及原owner字段；返回：原errno。
    /// 调用方必须在任何后续可失败操作之前已持有完整管道与安装租约，非零保证没有child。
    pub(super) fn birth(
        path: &CStr,
        channels: [i32; 3],
        pid: &mut libc::pid_t,
        owns_group: &mut bool,
    ) -> i32 {
        // 安全性：原owner与通道保活，C只在有效borrow输出中写合法PID及bool，不回调Rust。
        unsafe {
            diskgraph_macos_native_spawn(
                path.as_ptr(),
                channels[0],
                channels[1],
                channels[2],
                pid,
                owns_group,
            )
        }
    }
}
