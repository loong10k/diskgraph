# Windows 台式机失败隔离诊断

受测源码：`5fd11d6c0b190f88f0f81c212281ae7693e3efa3`，Git tree `a840a8c04550426fa90f9f58bc1213b57a9a956f`。使用台式机独立归档目录，不更改原 checkout 或其他构建进程。

完整原生任务 `wc_job_q3WmdwHEKPGawI7g` 已实际终态失败。foundation 为 380.454 秒、entry 为 643.252 秒，均 exit 101，均未触发各自 1800 秒期限，原 Job 退休确认成功。原回执及完整日志仍位于 `E:\workspaces\workspace-loong10k\diskgraph_native_5fd11d6c_20261009\native_validation_stable\workspace`。

已读取的 lib 结果：FFI 88 passed / 16 failed；store 386 passed / 2 failed / 8 ignored；MCP 223 passed / 10 failed。日志中的被捕获 panic 不能算作失败，以上来自实际 `test result: FAILED`。多数失败包含 `budget_exceeded`，部分 SSE 握手为 HTTP 503；不能据此认定已定位根因或判定全部为环境故障。

隔离任务 `wc_job_9m90W1ixYDjGKlTf` 已终态 exit 0。以下五项逐个执行，保持源码、原断言、原请求期限，均实际运行 1 test 且通过：

- store `reader_admission_tests::cache_configuration_busy_wait_uses_remaining_original_deadline`
- store `revision_ownership_reader_tests::terminal_sql_is_interrupted_under_original_deadline`
- FFI `native_reply_deadline_tests::captured_policy_still_allows_a_normal_authorized_query`
- MCP `history_budget_tests::encoded_socket_history_rechecks_both_grant_sides`
- MCP `sse_identity_deadline_tests::modern_socket_closes_and_joins_while_control_owner_holds_lock`

原生工具观察回执保存在 `docs/benchmarks/windows_desktop_5fd_native_2026_10_10/isolated_replay_observation.json`。单包执行会改变 Cargo feature 合并范围，因此仍需原完整命令诊断，不能仅据这五项将完整失败关闭。

首轮串行诊断 `wc_job_c4-7KH-fgfLaE53x` 的临时脚本把 `DISKGRAPH_WINDOWS_WORKER_QUALIFICATION` 错写成字符串 `1`，而真实原生测试要求当前 worker 的路径。该错误由诊断脚本引入，不属于产品源码缺陷。本轮已按精确 Job ID 停止，并实际观察到 stopped / terminal / exit -1；停止回执保存于 `docs/benchmarks/windows_desktop_5fd_native_2026_10_10/invalid_diagnostic_stopped.json`，不能用作验收。

修正版串行诊断任务 `wc_job_lBvuY8G8CiuXNph4` 已派发。五个夹具入口均在执行前检查绝对路径、实际文件及非链接，qualification 与受信 worker 路径一致。使用原 `run_phases`、原全部包和 all-targets，仅显式 `RUST_TEST_THREADS=1`；每阶段 1800 秒保持不变，回执标记 diagnostic_only。结果未完成，不勾选 Windows 验收。不用串行诊断替代产品实际并发查询、200k 文件、安装包、安全退出或其他生产门禁。

修正版进度：实际 FFI lib 已完成 `104 passed / 0 failed / 0 ignored`，耗时 211.57 秒。真实扫描器活动进度后的取消、无效控制、父端 EOF 三项均通过，原生 worker 返回相应错误并退出；三项测试包含夹具创建与清理共 145.72 秒，不将该总耗时当作取消延迟。store 正在执行，尚未读到整段终态。此时观察回执保存在 `docs/benchmarks/windows_desktop_5fd_native_2026_10_10/serial_progress_ffi.json`，其中阶段 `exit_code` 仍为 null，不能据部分成功关闭完整门禁。FFI 的原并行失败与串行通过均保留。

后续终态：foundation 实际 exit 0，耗时 849.938 秒，未超时，原 Job 退休确认成功；entry 已开始，尚未终态。完整基础日志为 143478 字节，SHA256 `ad929a3954db10d1a8252c7bbaa7cb5808ffa0ddf50872ef8e7aab1a3d62b08f`。原始摘要观察保存于 `foundation_summary_observation.json`，其 library_results 提取器会把 store 的嵌套子测试误列为父测试统计，不能使用该字段证明父测试数量；原阶段退出码、日志摘要及 VM 原记录不受该提取错误影响。

实际 Windows 节点窄读 VM 记录：20k 与 200k 两种节点规模均为 `selected=1 steps=59`、`selected=16 steps=944`。这是同一查询选择量的 SQL 执行工作量证据，不能代替 p95、内存或完整并发性能验收。

修正提取器后，完整 store lib 为 `388 passed / 0 failed / 8 ignored / 0 filtered out`，耗时 463.67 秒；忽略项不计通过。修正观察保存在 `foundation_summary_corrected_observation.json`，按实际 Cargo 运行段边界排除嵌套子测试统计。与原并行 `386 passed / 2 failed / 8 ignored` 对照，两项失败在该完整串行诊断中未复现，不能据此认定并行负载问题已修复。FFI 同样保留原 `88 passed / 16 failed` 与串行 `104 passed / 0 failed` 两种结果。

基础阶段完整原始日志已无损回传为 `docs/benchmarks/windows_desktop_5fd_native_2026_10_10/foundation.log.gz`。本机实际解压并验证原始 143478 字节和上述 SHA256，回执为 `foundation_log_receipt.json`；不使用文本转码或换行转换。

修正版串行任务现已实际终态失败：entry 触发原 1800 秒期限（1800.031 秒），原 Job 回收确认成功，整轮不通过。entry 日志 62551 字节，最后执行位置为 `git_scope_boundary_tests::unrelated_ancestor_sibling_activity_keeps_registered_route_valid`，此前 `git_resource_tests::git_head_and_upstream_resource_errors_cannot_be_missing_references` 断言失败，诊断为 probe deadline exceeded，并附带原 owner 保留的清理诊断。日志中的独立 `FAILED` 行此前未被简单摘要提取器识别；以完整尾部和阶段回执为准，不能声称 entry 无失败或 MCP 已通过。

针对以上两个位置，任务 `wc_job_fuPI9p2GbgRkcHfi` 保持原完整 entry Cargo 包与 all-targets 的 feature 合并范围，分别执行原 exact 测试，测试线程为 1。实际任务已 terminal / exit 0，两项各运行 1 test 并通过，分别耗时 18.531 秒和 16.891 秒（含 Cargo 及其他 target 的过滤执行）。每项独立诊断外层期限 180 秒，产品源码、内部预算与断言未改动，均未超时且原 Job 回收确认成功。该结果排除了两项在单独执行时必然失败的假设，不证明原整轮失败已修复。

随后任务 `wc_job_q4tXoreAk-aBbrpj` 直接运行原完整 entry 构建的 MCP lib 测试镜像 `diskgraph_mcp-c3a96ac32331cd5f.exe`，保持原 feature 合并、源码、断言和请求预算，仅设置测试线程为 2。实际日志为 `233 passed / 0 failed / 0 ignored / 0 filtered out`，测试耗时 85.09 秒；相对原默认并行 `223 passed / 10 failed`，该完整 lib 诊断未复现失败。观察保存在 `git_terminal_diagnostic_observation.json`。不据此关闭默认并行失败，也不把 lib 通过替代 all-targets、实际产品并发或整体生产资格。

该 MCP 诊断随后已实际 terminal / exit 0，未超时，原 Job 回收确认成功，整轮耗时 85.312 秒；测试镜像 SHA256 为 `fda0545b1e1dd512e281a166329cf1e968baae1b823949779bea1ea48242c4ac`。终态回执保存在 `mcp_two_thread_terminal_observation.json`。

完整 entry 两线程诊断 `wc_job_2GUmj0aBhqRQ9L7-` 正在执行原 CLI、Engine、MCP 的全部 targets（原 locked/no-fail-fast/nocapture，外层仍为 1800 秒）。使用独占 `entry_two_thread_diagnostic_v1` 输出目录，不改源码、夹具与产品预算，不复跑已完成的 foundation。当前仍未终态，不能计为通过；两线程结果即使成功也不自动关闭默认并行完整验收失败。

该两线程完整诊断已出现实际失败：`git_isolation_semantics_tests::ordinary_dirty_stash_and_upstream_match_real_git_without_source_writes` 在真实采样返回 `probe deadline exceeded`，并保留原 owner 与私有 Git 清理诊断。用例使用原默认整次 15 秒预算，没有提高预算或改断言。该证据说明不能把所有失败归结为默认高测试并发，也不能将 MCP lib 的两线程成功推广到完整 entry。任务继续按原 no-fail-fast 收集结果，尚未终态。

新增优化验收：在同一私有 Git 视图中，upstream 成功解析为有效 commit OID 时，不再单独启动引用存在性查询。SHA-1/SHA-256 都保留原 OID 格式校验；解析正常失败时，仍以原 `show-ref --exists` 区分明确缺失、已有损坏与接口错误；原执行期限、取消或输出预算错误必须直接传播，不能降级为缺失。改变的是成功路径的原生进程数量，不增加缓存、不跳过 HEAD 或源身份终检、不提高预算。该优化仍需本地回归及 Windows 原生验证，不能预先声称解决所有超时。

该优化的实际 TDD：修改实现前，新增三项测试失败、既有七项通过；实现后引用测试 10/0，补充非法成功输出与带 stderr 的失败不能归类为缺失后为 11/0。真实 Git 语义测试 19/0（包括 SHA-256、缺失/损坏 upstream、悬空引用、缺对象和 HEAD 变化），Clippy `-D warnings` 与两文件 rustfmt 检查通过。成功 upstream 路径从三个引用查询进程减为两个，仅这是已证明的性能变化，没有宣称墙钟改善或 Windows 超时已解决。

本机更广回归分别为实时采样 315 passed / 1 failed / 3 ignored、Git 242 passed / 1 failed / 3 ignored。唯一失败为 `git_indexed_identity_tests::indexed_git_directory_replacement_cannot_be_sampled_as_the_original_node`；单项诊断仍失败，实际位置为 `NativeScanEngine::open`（测试第 28 行），错误 `Business(Unsupported)`，发生在 Git 采样之前。当前 macOS 入口需要已有受信安装部署，本机缺少该夹具；未安装或绕过该条件，不计为整库回归通过。完整 Git 输出保存在 `docs/benchmarks/git_upstream_query_2026_10_10/macos_git_regression.log`。本机通过的窄回归不代替三平台新源码 CI 或 Windows 原生验证。
