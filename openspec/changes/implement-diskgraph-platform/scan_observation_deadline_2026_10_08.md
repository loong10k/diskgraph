# 扫描采样的控制锁与 fencing 期限

RT-06：ScanObservationGuard::check_now 原先通过无期限 Engine::control 等待共享锁，并由旧 with_job_fence 使用控制连接默认 busy 窗口取得 IMMEDIATE 事务。已耗尽的扫描期限不能终止这两处等待。

当前实现沿原 started + max_duration_ms 获取控制锁，实时状态 SQL 使用同期限的 read guard；退出后，以新 with_job_fence_until 沿既有 ControlWriteDeadline 验证完整 owner/租约/取消/scope/IndexWrite 和持久 job authority。未重用 Process 专用预检查，未嵌套 progress handler。事务回调不会重放，原控制连接配置在失败或展开时由既有守卫恢复。实时 scope 撤权只读标量，不重复拥有根定位和显示文本。独立图库已提交后控制提交失败仍须原回执恢复，不宣称跨库事务。

真实锁 TDD：旧源码下 4 项中 2 项失败，分别是共享控制 mutex 和另一个 SQLite 连接的 IMMEDIATE 写事务在原 50ms 预算后仍阻塞，直到测试于 300ms 释放锁。候选 4/0，保留 scope/grant 撤销、持久取消、fence mismatch 的原拒绝分类，并实际验证超时后原控制连接可再次取得 fence。夹具仅创建真实隔离控制/图库、排队和认领，不运行受管原生 worker；不能证明三平台完整扫描通过。

初始夹具因复用 scope 注册前策略快照而 PermissionDenied，修正后取得真实锁 RED。中间实现误用 Process 专用 fence 被有效正控拒绝，已改独立通用扫描 fence；首次负面测试错误期待 fence mismatch 的 StaleOwner，但原 cancellation_requested 已将该情况分类为 Conflict，修正断言后对旧源码重新取得最终 2RED/2GREEN。上述准备/实现失败不掩盖为已通过。

本机当前混合 worktree Store 全包 357/0/9 ignored（7 suites，包含既有用户 reader_observation_tests），Engine/Store 源码结构门禁 6/0 与 1/0、all-target Clippy、两包 fmt、OpenSpec strict 通过。没有运行完整 Engine/workspace，受管 worker 安装限制仍在。保留用户 Store lib 的 reader_observation_tests 声明，不将其带入本次提交。

旧 8db3617 的 CI 37740264224 已终态 14 成功/9 失败。Windows stable 的五项认领回归、MSRV 同五项及终态重复执行待 8dd428f 验证；MSRV 另在 expired_envelope_still_observes_live_authorization_without_renewing_data_deadline 的慢回调撤权优先断言失败，仍开放。Intel 授权正控超预算、macOS ENOENT 和 Windows 300 秒负载仍未关闭。原始终态、两份 Windows 日志及本机红绿证据见 `docs/benchmarks/scan_observation_deadline_2026_10_08/`。

同 SHA 原生验收尚未运行，不勾选平台或生产父项。
