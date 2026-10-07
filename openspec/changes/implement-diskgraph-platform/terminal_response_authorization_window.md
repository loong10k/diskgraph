# 响应终检的授权观察窗口 / Terminal response authorization windows

## 已确认的问题 / Confirmed regression

CI run 37605519602 at dc98227 fails the existing native partial-deadline response test on Linux ARM, Linux x86 stable/MSRV and Windows MSRV. Reader admission correctly rejects an expired deadline, but the response finalizer attempted to use that exhausted data deadline to observe authorization. An encoded prefix consequently could not reach its existing explicit incomplete/deadline response.

新的 reader 准入检查保持不变。修复限定在响应终检：初次真实归属与控制 SQL 有固定 50ms 观察窗口；所有能力回调在 SQL progress guard 外执行，允许结果须在固定 50ms 协作窗口内完成；回调后的持久授权与新鲜归属查询共用另一个固定 50ms 窗口。原数据期限不续期，过期状态仍返回 false。所有回调后复验各侧 scope/grant；真实撤权先于迟到允许拒绝。授权连接不交给数据消费者，控制锁竞争立即拒绝。

The adapter retains its existing cancellation gates. This finalizer introduces no cancellation parameter. Synchronous host callbacks and OS I/O are not hard-preemptible. Failed observations refuse all output rather than treating unobserved authority as permission.

## 本机证据 / Local evidence

- A new isolated fixture publishes a valid snapshot through the actual Store API, without requiring a scanner deployment. Against the original HEAD finalizer it fails with Store(BudgetExceeded), proving the behavior regression independently of scanner admission. Candidate: 1 passed, 0 failed.
- 同一回归验证原数据到期返回 false、持有控制锁时 BudgetExceeded、80ms 允许回调返回 BudgetExceeded、独立真实控制连接在慢回调撤权返回 PermissionDenied，以及随后静态撤权继续拒绝。
- Independent code review: APPROVE; independent architecture review: CLEAR. Reviews cover this local change, not overall production readiness.
- The original native FFI test cannot run locally without its trusted macOS scanner deployment: it fails before query execution with Unsupported. This is not counted as a behavior RED or native acceptance.
- Windows stable's separate COMMIT-lock fixture passes locally, but its original Windows failure remains unresolved. Its assertion now includes the actual result, elapsed time and lock witness; no deadline or acceptance condition is relaxed.

后续门禁：候选提交的真实 CLI/MCP/FFI 与 Linux/Windows/macOS CI；Windows COMMIT 失败原因确认；监督进程交接、前台有限退出与最终跨平台性能验收。此记录不勾选生产完成。
