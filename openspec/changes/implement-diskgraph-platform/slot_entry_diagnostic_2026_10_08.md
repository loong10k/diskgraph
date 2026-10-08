# 恢复槽失败观测与当前原生验收

本增量仅补充 RT 启动准入失败诊断，不关闭 PF-06、平台或生产就绪门禁。

8db3617 的 CI 37740264224 中，macOS stable 的 CLI scope add 在恢复槽 openat 返回 ENOENT，原 held 目录诊断为 held_path_same。该观察不足以确定失败时目录项状态。新增独立模块使用原目录 fd 和固定槽名执行 fstatat(AT_SYMLINK_NOFOLLOW)，只输出粗粒度目录项类型及独立诊断 errno。原错误在诊断前保存并直接返回；不重试、不删除、不修复或扩充容量域。元数据系统调用仍不承诺强制抢占。

本机验证：新增诊断测试 3/0；既有恢复域测试 7/0/1 ignored（其中父测试实际启动隔离子进程）；Engine source_layout 6/0；Engine all-target Clippy、Engine fmt 和 OpenSpec strict 通过。新错误保留测试首次因夹具目录未显式设为 0700 而 Unsupported，修正夹具后通过；不是生产行为 RED。全 workspace fmt 检查报告 vendored disktree 格式差异，未修改上游源码；不声称该检查通过。

隔离 syscall 探针：8 个实际进程、合计 80000 次固定四槽 openat，0 错误。探针没有实现恢复槽记录协议或受管 worker，不能据此排除 CI 故障，也不是生产验收。

同一旧 SHA 原生包日志：Intel x86_64-apple-darwin 的 stdio 18、HTTP 13、升级回滚 7、20k/200k 负载各 6 项通过。Windows 正式包在 300 秒门禁超时，记录夹具准备约 272 秒后进入 index，未获得完整结果。随后 900 秒独立诊断完成：夹具 220.338 秒、scan 193.097 秒、query p50/p95 34.498/41.89 ms、完整 200001 节点及 6/6 检查；总诊断 663.822 秒且环境清洁性未确认，不能替代正式验收。不得据该诊断放宽 300 秒门禁或宣称 Windows 就绪。

原始日志、探针和 Windows 诊断原回执位于 `docs/benchmarks/slot_entry_diagnostic_2026_10_08/`。三平台完整 CI 尚有任务运行，本增量尚未获得同 SHA 原生验收。保留全部现有任务勾选状态。
