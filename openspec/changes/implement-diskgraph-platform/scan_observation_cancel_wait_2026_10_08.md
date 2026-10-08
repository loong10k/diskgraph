# 扫描采样在共享控制锁竞争期间响应取消

前一增量 5dc825d 已约束原扫描期限，但 control_until 只检查单调期限。扫描采样遇到共享控制 mutex 长期竞争时，即使原 cancel flag 已设置或 keeper 已保存真实撤权错误，仍会等待锁释放或整个扫描期限。

新增真实竞争回归：测试线程先获得同 Engine 控制 guard；执行线程实际 try_lock 遇到 WouldBlock 后由线程局部一次性通知确认等待已经发生，再设置取消。旧实现直到 300ms 后释放锁才返回，实际行为 RED。候选采样在每次 1ms 锁等待前复核同一 cancel、原 keeper 原因、token 到期及原扫描期限；锁仍被持有时返回原停止原因。正常取消保留 Conflict，keeper 原拒权保留 PermissionDenied，不读取或改写数据库来伪造取消。

新增 cfg(test) 等待通知只报告真实 WouldBlock，不等待、不执行 SQL；不编入生产。旧 control_until 与当前采样等待点共享同一测试通知，因此 RED 来自实际不能停止，而不是测试未观察到等待。

本机采样预算/取消/权限/fence 回归 5/0；结构 6/0、Engine all-target Clippy、fmt、OpenSpec strict 通过。原受管 worker 和跨平台验收未包含在本组夹具内。误选的 job_stop_reason_tests 过滤实际执行 0 项，不作为验证证据。

本增量只解决共享 mutex 等待期间的协作停止。SQLite fence 仍沿原期限处理竞争，不能据此声明所有原生/SQLite 系统调用都可即时取消，也不关闭生产就绪或 PF-06 门禁。证据为 `docs/benchmarks/scan_observation_cancel_wait_2026_10_08/`。

当前 5dc825d 的 CI 37744394886 仍有实际任务运行，不包含 9441f41 的最终响应撤权修复及本增量；保留现有运行，待终态后推送本地候选进行原生验收。
