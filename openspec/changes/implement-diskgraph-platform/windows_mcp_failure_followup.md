# Windows MCP 回归失败复核

## 2026-10-09 剩余原生集成覆盖

同一 658bade、实际 worker 和当前源码 driver、默认测试并发下，真实 Git socket 目标为 3/1：发布正向路径的 runner 报 timeout（exit 7），任务实际 Failed，不能标为可重连完成。其余此前未执行的 9 个目标改用 Cargo --no-fail-fast 收集结果（不改变 libtest 并发或产品预算），41/0；其中 http_signal_shutdown 在 Windows 为 0 项，不计为平台信号验收。源码前后摘要一致，原回执与 stdout 保存在 windows_mcp_socket_658_2026_10_09.json、windows_mcp_other_contracts_658_2026_10_09.json。

同期 Windows CI 200k 正式端到端 300 秒验收仍失败；随后独立诊断完整 6/6、总计 640.468 秒，创建 224.201 秒、扫描 172.704 秒、清理 232.010 秒，查询 p95 50.149ms。扫描低于 300 秒不等于端到端通过。原始 ZIP、binary/source 摘要及各阶段日志保存在 docs/benchmarks/ci_37865556195_windows_load；诊断之前的正式进程退休状态未获确认，不据此宣称干净环境下性能通过。

658bade 的第一轮 MCP 默认并发 lib 测试为 229/2，错误均为真实历史 socket 正向响应 budget_exceeded。为定位原终检的阶段，后续显式启用既有 DISKGRAPH_QUERY_DIAGNOSTICS；没有改生产源、测试期限或并发设置。

历史组独立为 10/0，完整 MCP 第二轮 lib 为 231/0；两轮均无 authorization_budget_phase 输出。未复现不等于修复，不能据此定位先前失败至某条 SQL，也不能用诊断开关开启的通过覆盖默认构建的失败。源码前后摘要一致，原始 receipt 及日志摘要分别见 `docs/benchmarks/windows_history_phase_658_2026_10_09.json`、`docs/benchmarks/windows_mcp_full_phase_658_2026_10_09.json`。

完整第二轮随后执行到了原先因 lib 失败而未执行的集成测试：部署合同 6/0，公共配置兼容 1/0，持久请求权限 4/0；git_evidence_contract 为 10/1，authenticated_git_job_execution_is_reconnectable_with_the_actual_receipt 在实际 run_job_strict 返回 Business(Timeout)，Cargo 退出 101。HTTP panic RAII 用例的预期 panic 后测试已通过，不能作为失败原因。后续测试未获执行通过证明。

下一步针对实际 Git job 的执行耗时与清理生命周期取证，不放宽 15 秒采样期限，不把 release、单个测试或其他提交的成功合并成当前完整验收。历史正向期限的偶发失败及完整 Engine/CLI 回归仍开放；本记录不关闭任何生产父项。

同期 CI 37865556195 的其他失败边界分别记录于 `docs/benchmarks/ci_37865556195_failure_boundary_2026_10_09.json`：macOS ARM stable 的 Store 原墙钟门禁失败前，夹具 100ms sleep 实际已让 SELECT 开始于 247.528ms，随后 2.375µs 返回 BudgetExceeded，不能解释为 SQLite 续期；240ms 原门禁保持不变、验收仍失败。Linux stable 的历史 release 基线在 200k wide round1 退出 101，尚无完整配对性能结果，不能用当前包验收或 Rust 测试通过替代该比较。
