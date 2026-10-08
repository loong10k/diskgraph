# 提交后 WAL 维护与扫描租约（2026-10-08）

范围：RT-01，只读 CLI/MCP 扫描发布；不改变 30 秒租约、5 秒续租、提交前认证/取消/租约检查，也不放宽 300 秒负载验收。

## 当前证据

Windows CI 37731786365 的完整 200k 诊断绑定 1fb077c，三个 release 镜像 SHA 已记录，三个脚本按 Windows CRLF 的源码 SHA 与 Git 对应提交核验一致。夹具准备 222.771 秒；首个 worker 7.943 秒完成，转换 10.995 秒、staging 79.723 秒完成，发布窗口 79.802 至 156.491 秒。单个 index 调用内随后又执行一个 worker，第二次发布没有观察到完成，内层 index 在 300 秒超时。

诊断没有 job/fence 标识，也没有原发布窗口内部细分，因此重复执行与租约过期的因果关系、checkpoint 与 SQL/commit 各自占时仍待原生验证。前置正式负载的退休状态未证明，不能称干净环境或生产验收通过。

## 实施与验证

源码明确显示 checked 发布在图事务 commit 后、退出控制库 fence 回调前执行显式 TRUNCATE checkpoint；它占用控制锁，且不参与提交前租约末检。回归测试通过真实 SQLite authorizer 观察 PRAGMA，修复前因 fence 内实际执行 checkpoint 失败，修复后确认 checked 发布只提交，可信旧发布入口仍执行维护正控制。

Engine 改为先持久结算任务终态、释放控制锁，再执行显式 checkpoint，维护失败不回写已提交任务状态。可信旧入口继续同步维护，容量门禁和打开时大 WAL 恢复保持。诊断模式新增无路径/主体标识的 checkpoint 起止，供下一轮 Windows 验证。

本机发布回归 5/5、WAL 回归 5/5、源码规范 6/6、Engine/Store all-targets Clippy 通过。当前工作区 Store 库串行 320 通过、5 忽略，其中包含保留的用户未提交 reader_observation 测试，不能称提交精确测试集。证据见 docs/benchmarks/post_commit_checkpoint_2026_10_08/。

未完成：当前修复的原生 Engine/CLI 行为与 Windows 200k 正式验收；SQLite 隐式 auto-checkpoint/commit 仍可能较慢；图库提交与控制库结算间的崩溃窗口仍需独立持久回执协议。本项不关闭生产就绪父项。
