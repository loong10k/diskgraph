# Windows MCP 回归失败复核

658bade 的第一轮 MCP 默认并发 lib 测试为 229/2，错误均为真实历史 socket 正向响应 budget_exceeded。为定位原终检的阶段，后续显式启用既有 DISKGRAPH_QUERY_DIAGNOSTICS；没有改生产源、测试期限或并发设置。

历史组独立为 10/0，完整 MCP 第二轮 lib 为 231/0；两轮均无 authorization_budget_phase 输出。未复现不等于修复，不能据此定位先前失败至某条 SQL，也不能用诊断开关开启的通过覆盖默认构建的失败。源码前后摘要一致，原始 receipt 及日志摘要分别见 `docs/benchmarks/windows_history_phase_658_2026_10_09.json`、`docs/benchmarks/windows_mcp_full_phase_658_2026_10_09.json`。

完整第二轮随后执行到了原先因 lib 失败而未执行的集成测试：部署合同 6/0，公共配置兼容 1/0，持久请求权限 4/0；git_evidence_contract 为 10/1，authenticated_git_job_execution_is_reconnectable_with_the_actual_receipt 在实际 run_job_strict 返回 Business(Timeout)，Cargo 退出 101。HTTP panic RAII 用例的预期 panic 后测试已通过，不能作为失败原因。后续测试未获执行通过证明。

下一步针对实际 Git job 的执行耗时与清理生命周期取证，不放宽 15 秒采样期限，不把 release、单个测试或其他提交的成功合并成当前完整验收。历史正向期限的偶发失败及完整 Engine/CLI 回归仍开放；本记录不关闭任何生产父项。
