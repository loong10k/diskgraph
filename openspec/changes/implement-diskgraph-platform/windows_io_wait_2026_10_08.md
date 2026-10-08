# Windows 扫描驱动有限事件等待

延续 PF-06 的非阻塞轮询与原 owner 合同。当前 Windows stable 忙碌输出回归完整耗时 1.6010678 秒，未满足原 1 秒断言。源码中 `ReadFile` 返回 `ERROR_IO_PENDING` 时本轮没有已消费字节，现有驱动随后无条件 sleep 最多 20ms；即使 I/O 在 sleep 前后已经完成，也不能提前唤醒。这是明确存在的延迟路径，但不是所有该次测试耗时的已证明唯一原因。

验收要求：

- Windows 待机等待原 stdout/stderr/control 的 pending OVERLAPPED event，任一完成立即回到原 poll；等待最多原 20ms，且不超过原请求剩余时间。
- 仅读取原 owner 的事件，不复制/关闭句柄，不提交新操作，不改变 OVERLAPPED 地址，不将事件唤醒当作数据、EOF、End 或正常退出。
- 已消费输出继续零等待；无 pending I/O 保持有限 idle backoff。Unix 行为保持。
- Wait API 失败传播原错误并走现有 owner 处置路径。每次唤醒后原授权、取消、fence、预算和双管道公平读取检查保持。
- 原忙碌输出 1 秒断言、无输出不允许提前退出和安全清理测试保持；补充原生 event 立即唤醒、过期不访问句柄、未完成 event 和 wait 错误测试。

Windows 原生测试只能由对应 CI 验证；本机无法运行 Windows 测试，不声称已观察到原生 TDD 红灯或性能收益。不关闭 200k 全流程 300 秒、身份授权稳定性或整体生产门禁。

API 依据：[Microsoft WaitForMultipleObjects](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitformultipleobjects)。等待期间句柄始终由被借用的原 child 持有，最多三个不同的 manual-reset event，不等待 mutex，不使用无限等待。

## 本机验证

macOS ARM64 使用本轮 Cargo 实际构建的独立 `scan_worker_driver_fixture`，明确仅为协议/进程退场夹具：

- `scan_worker_driver_additional_tests` 5/5 通过；原忙碌输出 1 秒断言通过，完整 natural-exit 耗时约 46.94ms；取消、panic 清理和无输出不提前交付均通过。
- `scan_worker_driver_tests` 8/8 通过；保留原响应总额、双管道背压、完整控制帧后取消、检查器资源释放与实际 leader 退出要求。
- `source_layout` 6/6 通过，检查所有平台生产源码 AST；不等于 Windows 编译或运行。
- Engine 全 targets Clippy `-D warnings`、Engine fmt、OpenSpec strict 与 diff 空白检查通过。

首次本机编译发现 wait 原生错误误用了 `from_child`（其参数是带检查点的 spawn 错误）；已改为现有 `From<ChildError>`，随后上述构建与检查通过。该编译失败不是 Windows 行为红灯。新增四个 Win32 event 测试未在本机运行，Windows 全量验收保持未完成。
