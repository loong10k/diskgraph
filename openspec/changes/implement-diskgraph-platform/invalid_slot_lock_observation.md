# 损坏恢复槽的锁优先观察

事实源：implement-diskgraph-platform，PF-06。生产协议与代码不变。

旧提交 5c85a48 的 macOS stable job 112726538801 唯一全量失败是 `invalid_records_are_refused_without_modification`。原认领先取得文件独占锁再校验正文；并行 Unix fork 可暂时继承该锁。测试不能在仍然 Busy 时要求已观察到 InvalidRecord。

修正只在测试的原固定期限内重试 Busy，最终必须精确 InvalidRecord；成功认领、到期或其他错误仍失败，正文始终保持原样。新真实子进程回归持有损坏槽的原文件锁，父侧确认 Busy 后通知释放，子进程延迟 100ms 实际关闭原文件。旧立即断言真实 RED：Some(Busy)；新观察 helper GREEN。正常路径实际确认子进程成功退出，异常路径由原 Child guard kill/wait；这不是产品有限退出资格。

本机 macOS arm64：recovery_slot 全套 9 passed、0 failed、1 ignored。ignored 子夹具由正常测试显式执行；新子夹具实际 1 passed、0 failed、0 ignored。目标 Clippy 与文件格式检查通过。独立 code-review APPROVE、architecture CLEAR。

Linux／Windows／macOS Intel 及候选提交 CI 尚待验证。睡眠、系统锁和 kill/wait 均为协作式边界，不承诺严格墙钟上限。此修正不清除 ACTIVE、不替代监督进程接入，也不关闭生产总门禁。
