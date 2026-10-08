# 实时权限原子观察与当前性能证据

范围仍为 macOS/Linux/Windows 只读 CLI/MCP。整体未完成，不勾选生产父项，不启用危险写工具。

## 真实撤权竞态

原 `ControlStore::live_permission` 先读取 scope，再读取 policy，最后读取精确 grant。真实双连接回归在 policy SQL 准备时由独立连接提交 scope 撤销，旧实现仍返回 `Some(true)`，目标行为 RED 已复现。该问题是单次授权观察拼接，不据此声称外层所有请求均能绕过撤权见证。

现将 scope、policy 和精确 grant 合并到一条参数化 SQL，同一 WAL 观察返回 `Some(false)`。不缓存授权、不续期、不复用旧读事务。保持缺失 scope、未发布策略、撤销范围优先级、策略必需字段损坏、版本及精确主体/权限/范围语义。正常有策略的调用由三个 SQL 读取变为一个；未以此声称已修复 CI 间歇预算失败。

本机 ARM macOS 验证：双连接目标 RED→GREEN；权限组 7 通过、1 个手工性能诊断忽略；Store 全库 342 通过、5 忽略（含用户未提交的两个 reader observation 测试，不能投影为纯提交测试清单）；Engine 关系/历史末段独立撤权回归 1/1；Store source_layout 1/1；Store all-targets Clippy、Engine 关系测试 Clippy、项目格式门禁和 OpenSpec strict 均通过。误用包含上游 vendored 源码的 `cargo fmt --all -- --check` 会报告上游格式差异；没有修改上游，正式项目排除 vendor 的格式门禁通过。

补上 `PreparedStagingNode` 类型来源文档。此前遗漏 Store source_layout 门禁导致当前 CI 规范失败，修正后本机该门禁通过。

最新 `cfaf939` Linux MSRV 的 escaped impact 回归确实因 `BudgetExceeded` 失败；macOS stable 的 history quality 矩阵也因 `BudgetExceeded` 失败，其能力回调仅微秒，故不能将失败都归为文档或能力回调缓慢。保留原失败，escaped impact 复用已有定长授权回调诊断，原断言和所有期限不变；macOS 失败隔离增加上述两例，隔离通过不能覆盖 workspace 原失败。本机真实扫描集成入口因未提供受信部署返回 Unsupported，未进入目标业务断言，不作为该 CI 失败的复现或修复证明。

证据：`docs/benchmarks/live_permission_atomic_2026_10_08/`。

## 正式 Linux 配对结果

工作流 `37757218636` 在 `cfaf9395777e91bd913deccf2e34b11c30490ae3` 成功结束。原固定基线 `2a2f8281f9f211b6632bdb26afdcd4a4fb21a44d` 保持不变；20k/200k 宽目录、300 层深目录各两轮交替执行，原始产物摘要已核对。没有重标旧 SHA，也没有更换基线掩盖以前失败。

| 夹具 | 基线扫描秒（两轮） | 候选扫描秒（两轮） |
|---|---|---|
| 20k 宽目录 | 3.5673 / 3.4753 | 3.2468 / 3.2563 |
| 200k 宽目录 | 36.8858 / 34.7870 | 33.1659 / 32.6240 |
| 300 层深目录 | 0.1245 / 0.1139 | 0.1411 / 0.1947 |

200k 窄候选查询 p95：基线 1.349/1.179 ms，候选 2.307/2.362 ms；宿主扫描高水 RSS 由约195.6–195.7 MiB增至207.4–207.5 MiB，候选另有约54.7–55.7 MiB已回收子进程高水。父子高水不是同时 RSS，也不是严格内存上限。当前结果说明宽目录扫描较快，同时存在候选查询、宿主内存和深目录退化观察，不能宣称性能整体优化完成。

原始 JSON 和小型日志保存于 `docs/benchmarks/paired_linux_cfaf939_2026_10_08/`，`preservation.json` 明确未入库的原始二进制/归档材料及已保存文件摘要；完整原始 artifact ID `11540962534`。单一 Linux runner、两轮、未清缓存、不同原生宿主适配器，不能替代其他平台 SLO、打包 RSS 或长期运行验收。

监督恢复、真实 macOS FileProvider、查询预算稳定性、Windows身份、应用归属产品接线、Windows200k正式300秒门槛及同SHA全平台门禁继续开放。
