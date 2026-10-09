# 终态归属查询的调度与原期限验收

增量澄清 SC-04/Q-08：锁竞争仍不得续期。真实 rollback-journal 排他锁、150 ms 原期限及正常未过期分支的 240 ms 外部耗时断言保持。准备阶段的 sleep 可能被 OS 调度延后；若实际调用 SELECT 前原期限已过，必须精确返回 BudgetExceeded，不接受 busy/interrupted 作为等价成功，不将调度等待称为 SQLite 重试。生产连接继续 busy_timeout=0，未修改生产时钟或期限。

失败证据：b355ec80 CI 37888266414、macOS ARM job 113683090046，admission 102.5 µs、before_select 248.949 ms、select 2.667 µs、total 248.951625 ms，实际结果 BudgetExceeded。旧断言把查询前延迟误报为续期。此修复只修正该测试的归因，不解释另两项原生查询超时，不关闭完整 CI、长期运行或监督恢复父门禁。

验证须覆盖实际过期分支与实际未过期锁竞争分支。到期拒绝不是成功授权；不能放宽期限、降低真实输入规模或忽略平台失败以取得绿色。

本机结果：恢复原无条件耗时断言并提供250 ms准备阶段，目标测试真实失败（exit101）；修正后100/250 ms两轮分别返回DatabaseBusy/BudgetExceeded。归属组9通过、1忽略；Store工作区全目标400通过、12忽略，包含既有未提交reader_observation测试，不是精确commit全workspace资格。Store Clippy、CI相同十个项目包格式检查、diff检查通过；vendored副本不修改。四个原错误层级的Scenario标题已修正并经真实delta解析器识别；严格OpenSpec仍有六条既有长Requirement警告，没有宣称通过。证据见`docs/benchmarks/terminal_busy_scheduler_2026_10_09/`。

当前1f4074fb CI37901867819另有macOS ARM HTML撤权测试与Intel历史兼容性测试返回BudgetExceeded；Intel日志记录terminal_ownership_sql约79.9 ms、整个1000 ms历史请求约89.1 ms，独立终态授权窗口耗尽。本项测试修正不解释或关闭这些失败，不放宽生产窗口，不仅因下一轮绿色就删去原失败。Windows台式机正式200k仍300秒超时；独立900秒诊断完成526.891秒，创建145.839、扫描280.626、回收95.313秒，6/6正确性检查。诊断不替代正式门禁，原生命周期及平台父任务不勾选。

后续实际修复：控制库的SQLite配置准备与回调安装耗尽原窗口时，在进入消费者前精确返回BudgetExceeded，并仍清理进度回调、还原busy_timeout。没有刷新生产期限。真实SQLite authorizer延迟配置准备的回归测试在旧行为下失败（expired consumer was executed），修复后的Store全目标401通过、12忽略；Clippy通过。该本机结果包含既有未提交测试，并非完整精确提交资格，证据为control_admission_*文件。

严格OpenSpec现已通过：四个Scenario标题恢复正确层级，六条原有完整约束逐字保留在对应验收Scenario内，未删除或放宽要求。早先六条警告记录保留为历史结果，约束保全检查见spec_constraint_preservation.json。

1f4074fb CI最终23项中21通过、2失败，两项macOS失败仍未关闭，五个原生只读包任务均通过。Windows台式机同E卷、仓库外目录对照已结束：stdio18/18、HTTP14/14、20k6/6，200k仍300秒超时；创建阶段180.656秒，随后进入索引，未取得完整200k报告。路径改变只是输入对照，不是代码优化收益，详见windows_desktop_1f4074fb_2026_10_09/standalone/。长期运行、监督恢复及平台父门禁保持未完成。
