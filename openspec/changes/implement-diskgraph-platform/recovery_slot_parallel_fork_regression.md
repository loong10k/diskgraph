# 原生槽位回归的并行 fork 观察竞态

## 验收行为

预留超时、预留 owner 关闭或实际活动 child 死亡后，持久 RESERVED/ACTIVE 不得被当作 CLEAN。新认领必须最终返回 Unconfirmed，任何成功认领、其他错误或超期均失败。活动原锁仍持有时必须返回 Busy 的现有断言保留。

## 已取得证据

- 当前基线：11dd9588f21c65e629f23b46ba44ca2c08366016。
- CI 37589701883 / Linux Rust 1.97 job 112687983812 的 expiry_during_reserved_phase_preserves_unconfirmed_record 失败；原 RESERVED 内容断言通过，但原日志未提供实际认领错误。
- 原生 Linux ARM64、固定 Rust 1.97 镜像 rust@sha256:874db0adb75ee22be0fa0ba19dec70270456cd3db0f9385c06eaea33f09d4248：当前基线归档叠加错误诊断，在并行重复测试中实际复现 Some(Busy)。单项调整后同组 dropping_reserved_owner_does_not_fake_completion 也复现即时 Unconfirmed 断言失败。
- 最终测试夹具共用固定绝对期限的 Busy 观察循环；最终严格要求 Unconfirmed，没有改动产品 acquire、状态编码、锁或容量策略。
- 最终 Linux 同组 8 通过；同一编译产物以 --test-threads=16 重复 100 轮全部通过。子进程 helper 仍默认 ignored，由父测试实际启动，不是跳过资格行为。

## 限制

并行 fork 到 exec 之间短暂继承其他测试的原锁，与实际 Busy 观察及同组子进程启动源码相符；没有用内核追踪证明具体 fork 时序。该修复容忍短暂安全拒绝，绝不允许认领成功或永久 Busy 被视为通过。Windows 当前基线 CI 仍在运行，最终夹具的 Windows 原生验收尚未完成。此项不关闭可信监督启动链或总体生产门禁。
