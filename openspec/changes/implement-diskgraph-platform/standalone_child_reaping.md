# 独立入口的子进程回收信号策略

沿用 RT-08/PF-06，范围只含 Linux/macOS 独立 CLI 与 MCP 的启动。继承 SIGCHLD=SIG_IGN 或 SA_NOCLDWAIT 的进程可能被内核自动回收子进程，导致原 wait 无法取得退出事实；不能用旧数值PID补杀，也不能声明资源已确认。以隔离真实子进程验证该前置条件，不改变测试父进程的信号配置。

独立程序在解析参数、创建线程/Engine/runner/companion之前建立 SIGCHLD默认处理与清空SA_NOCLDWAIT。准备失败在启动前拒绝，不出生数据库或worker。不在Engine构造或FFI隐式修改宿主信号；显式库接口仅供完全拥有进程的启动入口调用，必须早于任何子进程与线程。Windows不调用POSIX策略，也不以本机结果声明Windows回收资格。

该修复不替代独立监督进程、原owner恢复、ACTIVE/CLEAN或异常死亡保护，也不清理旧未确认记录。需证明三种继承状态（IGNORE、NOCLDWAIT、两者）均能实际wait自己的新子进程；库内隔离正控不等于真实扫描或MCP平台验收。

## 本机验证（2026-10-10，macOS）

- 将准备函数临时替换为 `Ok(())` 后运行 Engine 隔离测试，实际失败于原子进程 wait：`No child processes`；恢复实现后三种状态实际 wait 通过。夹具在常规枚举中标记 ignored，但父测试显式启动并验证其完整执行。
- 真实 Cargo CLI/MCP 伴随程序测试 2/2 通过：拒绝无认证远程启动、退出码 6、未创建数据目录。CLI 接入前该测试同样通过，故只作为集成回归，不声称入口层已获得独立行为红灯。
- Engine、CLI、MCP 全目标 Clippy（拒绝 warnings）、修改文件 rustfmt 与 OpenSpec strict 检查通过。Cargo.lock 仅新增 CLI 对已有 libc 的依赖边，不升级包。
- Linux/Windows 当前提交的平台 CI 尚待执行；本机结果不替代扫描、异常退出、独立监督或跨平台验收。
