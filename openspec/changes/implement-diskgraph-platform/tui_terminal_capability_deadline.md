# TUI 终检允许结果提交期限修复

2026-10-07，属于现有 implement-diskgraph-platform / Q-08；不关闭生产总门禁。

源提交 a08a5b1 的 Linux stable CI job 112656311547 中，真实测试 expired_partial_frame_cannot_renew_a_slow_terminal_authorization 返回 Ok(()) 并失败。此前分离宿主回调与 SQLite guard 时，允许结果缺少终检提交期限，120ms 回调可以提交到期 partial。

修复在能力回调之前生成一次50ms提交期限；该期限覆盖回调及其后授权/归属核验。SQL仍使用各自原绝对执行窗口，先观察实际撤权及新鲜归属，拒权优先。全部授权通过之后，迟到的允许结果返回 BudgetExceeded，不提交终端缓冲。原数据期限不刷新；同步回调仍不能硬抢占。

macOS 原失败用例实际1/0，Engine all-target Clippy -D warnings通过。Linux ARM64 隔离环境运行 TUI 期限组10/0、runtime_revision_authorization 1/0（包含28种真实隔离模式）及 scope_authorization_projection_tests 5/0；恢复原 recovery_slot 模块入口，排除未提交 namespace 候选后重跑上述16项仍全部通过。压缩日志见 docs/benchmarks/supervisor_startup_admission_a08/tui_capability_deadline_isolated_green.log.gz，CI RED 摘录同目录 tui_capability_deadline_ci_red.txt。两条独立审查分别 APPROVE / CLEAR。此证据不替代 Windows、macOS Intel 或同 SHA 全量 CI；监督状态域候选与未接线入口 RED 不包含在此次提交。
