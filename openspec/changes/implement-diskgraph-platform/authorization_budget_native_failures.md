# 原生授权预算失败：2026-10-10

本记录沿用 Q-02、SC-04，不修改 50ms 授权窗口、查询期限、错误优先级或测试并发。父任务尚未通过。

源码 `ba2aa6ec2e6543fd11d8c71432f0c1c7c7cd1f20` 的 CI run `38046631229`：五个原生只读包均成功，每个执行 stdio 18、HTTP 14、升级回滚 7、20k/200k 负载各 6 项。GNU 两个架构的最终镜像最高需求为 glibc 2.17。包验收成功不能覆盖完整测试失败，也未证明旧 glibc 2.17 userland 已实际运行。

macOS Intel job `114197458786` 的 `history_preparation_budget` 为 9/1；普通正向比较被预算拒绝。原始诊断确认 `terminal_reader_open` 墙钟 89955μs、线程 CPU 1139μs，超过原 50ms 窗口。日志不能进一步区分调度与阻塞 I/O；不可将其称为字节额度耗尽。ARM 本机相同整个集成目标保持默认调度通过 10/10，不替代 Intel 失败。

Windows MSRV job `114197458801` 的 FFI lib 为 103/1。`growth_and_candidates_do_not_decode_unrelated_nodes` 在 growth envelope 断言处得到 `budget_exceeded`，具体阶段尚无证据。已补成组终检的调试诊断：首连接、归属 SQL、初次控制 SQL、末段观察及末段期限；保留原结果和撤权优先级，release 不启用。未声称此诊断修复了失败。

候选验证：真实递归控制 SQL 超时 1 项、末段能力/撤权回归 14 项、诊断结果保留 6 项、Engine lib Clippy 与 Engine fmt 通过。CI 合同新增整目标默认调度观察；旧提交工作流实际缺少该观察而失败，候选 5 项通过。后续原生失败观察保留原 workspace 失败，不用串行单用例或重跑成功替代它。

当前制品另在已有 Linux ARM 容器中完成 20k/200k 负载各 6/6；20ms 进程树 RSS 采样峰值分别 33944/183324KiB。该观察使用当前包、隔离数据库、固定容器资源，不代表严格 RSS 上限、Windows/macOS 原生 RSS 或配对性能提升。

证据位于 `docs/benchmarks/readonly_ci_ba2_2026_10_10/`、四组原生包证据目录、`release_rss_ba2_2026_10_10/` 和 `grouped_authorization_diagnostics_2026_10_10/`。有限前端退出、完整原生同源码门禁及其他尚未验收的平台能力继续开放；不勾选父任务，不归档变更。
