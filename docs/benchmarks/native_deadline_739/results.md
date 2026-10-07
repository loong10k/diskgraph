# 同出生私有 IPC 的原期限桥接子组件

正式规格：implement-diskgraph-platform/frontend_recovery_supervisor.md。时间材料仅用于同一 boot、已认证本机同出生私有通道；不认证 peer，不作为远程客户端预算或跨机器凭据。新增 API 未接入 CLI/MCP；不据此宣称监督通信、有限前端退出已完成。

先读原生计数后扣原 Instant 剩余时间，存绝对计数截止值。采用端先采本地 Instant，再读实际原生计数，扣除传输/排队和精度余量，仅接受同域/版本且剩余量不超过明确策略上限；过期、溢出、域不符及未知字段均拒绝。每次重复采用都扣实际已消耗时间，没有 Duration 刷新回退。

本机 macOS arm64 新接口的错误相对预算原型 RED 2/3：真实另一个进程错误采用已过期材料（退出53而非51）、排队后期限被延长、任意未来截止值被接受；绝对期限实现 GREEN 5/0。单独 ignored 的 child 入口仅供真实父测试显式调用，不能计为未执行平台通过。源码门禁 6/0，Engine all-targets Clippy/fmt/OpenSpec 严格验证通过；Python suite 245/0（已有 mock 文件句柄 ResourceWarning）。

Linux 真实 CLOCK_MONOTONIC 时间域以 nsfs 原 FD device/inode 绑定，采样前后核对；macOS 使用 CLOCK_UPTIME_RAW；Windows QPC 按真实频率整数换算，并扣向上取整一 tick 加一纳秒余量。依据：[Rust 1.97 Unix 时钟](https://github.com/rust-lang/rust/blob/1.97.0/library/std/src/sys/time/unix.rs)、[Rust Windows QPC 精度](https://github.com/rust-lang/rust/blob/1.97.0/library/std/src/sys/pal/windows/time.rs)、[Microsoft QPC](https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps)、[Linux 时间命名空间](https://man7.org/linux/man-pages/man7/time_namespaces.7.html)。

CI 新增隔离 Linux time namespace 实际拒绝门禁：只在 disposable github-hosted Linux 显式执行 ignored 父测试，sudo timeout 管理本次子进程组，unshare 创建临时 namespace 并增加该 namespace 的 monotonic offset，不改宿主时钟；父/子须证明真实 nsfs domain 改变并以 Conflict 拒绝，退出码、正向标记及实际测试数量均验证，原日志 always 上传。不在本机运行、不接受 unsupported/零测试为通过；此门禁及 Windows/macOS Intel 实测仍待新提交 CI。

receipt.json 绑定当前源码及原日志 SHA256。真实监督出生、私有对端认证、容量命名空间、原公开 EOF、前台/后台生命周期和全平台生产门禁仍打开。
