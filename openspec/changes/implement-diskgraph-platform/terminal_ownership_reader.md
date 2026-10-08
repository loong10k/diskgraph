# 原期限内的独立归属窄读

保留 SC-04 原终检 50ms、每次新鲜连接、过滤归属视图与非缓存授权契约。仅将终检专用连接与通用节点查询配置分离，不复用消费者连接或旧 WAL 事务，不扩大期限，不改变撤权优先级。既有 terminal_narrow_reader_experiment.diff 仅为未通过编译的候选，不能作为当前完成证据。

验收：实际 WAL 消费者旧事务仍看到旧权限时，新连接必须拒绝已隔离 revision；同一个独立连接两次 SELECT 之间的提交必须可见；原路径数据库替换后每次新打开必须检查替换后归属，旧连接不得作确认；过期准入不创建数据库，真实长 SQL 必须被原期限中断，忙等待逐 SQL 扣除剩余窗口；缺失/坏视图明确失败。

终检窄读只提供布尔归属接口，没有事务或通用节点入口。release 配对测量必须包括连接打开、SQL 与关闭，报告 p50/p95 与实际工作量，不把少两条配置 SQL 推断为全平台预算问题解决。完整原生安全回归与 CI 仍独立验收。

## 2026-10-09 原生结果与修正

Windows Rust 1.99 MSVC release、200 对交替样本：通用连接 p50 1.1418ms、p95 1.7342ms；初版窄读 p50 1.1595ms、p95 1.7715ms。该测量没有证明性能收益，不得宣称已解决 50ms 终检在并发负载下的失败。

初版按每条 SELECT 的剩余窗口重设 busy_timeout，macOS 回归通过，但 Windows 真实排他锁测试实耗 411.03ms，超过原 150ms 窗口。SQLite 默认 busy 回调累计请求 sleep 时长而非单调时钟实耗；因此终检专用连接改为 busy_timeout=0，锁竞争立即返回真实 BUSY，由既有引擎映射为预算失败，不重试、不自旋、不刷新期限。正常长 SQL 仍受原 VM 期限保护，同步打开及操作系统调度仍非硬实时能力。

初版 Windows：Store 6 通过/1 失败/1 忽略，Engine 撤权 6 通过，终检 13 通过/1 失败，MCP 224 通过/7 失败，CLI 84 通过/4 失败。保留这些失败；修正后的原生验证尚待完成。此变更和微基准均不替代全量并发、跨平台及完整 200k 工作负载验收。

禁止 busy 重试后的 Windows 对应源码指纹与命令见 `docs/benchmarks/ownership_reader_2026_10_09/windows_zero_busy.json`：Store 7 通过/1 忽略，恢复协议 9 通过，终检 14 通过，CLI 88 通过；MCP 229 通过/2 失败，失败为 impact/candidates 编码后到期的部分诊断。未把一次不同负载结果的失败数量减少解释为统计显著性能收益。全量 Engine 重测与 MCP 阶段诊断继续执行。

随后原默认并发 Engine 全量在 600.016s 达到验证外壳期限，未产出完整 suite 结果。大量空文件夹具先在原 120s 构造期限内失败，原恢复责任继续处理目录清理，输出 `probe cumulative output byte limit exceeded`。外壳结束原测试树不能证明产品有限退出或原目录回收。保留失败与原源码/命令记录：`windows_full_engine_timeout.json`。

MCP 开启阶段诊断的默认并发全量为 230 通过/1 失败，失败换为 related 编码到期的部分诊断；原终检 ownership SQL 实耗 39.746ms 后整体原窗口已耗尽。不同运行失败集合变化，仍表明并发稳定性门禁未通过，不能按最后一次较少失败数量关闭问题。原 50ms、15s Git 采样及完整 Windows 200k 的 300s 门禁均保留。

CI 37849885654 的 b510f823 macOS MSRV Store 完整组为 355 通过、1 失败、8 忽略；失败仍为 a_later_busy_select_uses_only_the_original_remaining_window，含准备与 100ms sleep 的实耗 246.773333ms，超过原 240ms 测试总门禁。当前连接已禁止 busy 重试，单一总耗时尚不能区分准入/夹具锁/调度延迟与 SELECT 本身；不据此宣布原因已修复。增加四阶段原单调时钟诊断，保留 150ms 原期限、100ms 实际等待、240ms 总门禁及断言，不改变产品 50ms 期限或测试并行数。macOS 对应复验保持开放。

最终诊断在查询返回时保存原实耗，再输出阶段记录，避免日志调度增加被测操作时间。Windows Rust 1.99 实际目标测试 1/1：准入 0.563ms，持锁 0.624ms，SELECT 前 100.929ms，SELECT 18.772ms，总 119.701ms，返回真实 DatabaseBusy；当前 Engine/Store all-target Clippy 及格式通过。证据 `docs/benchmarks/windows_private_creation_9d09e1c_2026_10_09/native_final_busy_phase.json`，任务 wc_job_f1MOeIgeBZdCaCSs。该隔离结果不消除 macOS MSRV 的完整组失败或原并发稳定性问题。
