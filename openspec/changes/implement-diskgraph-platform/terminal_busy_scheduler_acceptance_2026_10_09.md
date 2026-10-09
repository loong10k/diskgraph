# 终态归属查询的调度与原期限验收

增量澄清 SC-04/Q-08：锁竞争仍不得续期。真实 rollback-journal 排他锁、150 ms 原期限及正常未过期分支的 240 ms 外部耗时断言保持。准备阶段的 sleep 可能被 OS 调度延后；若实际调用 SELECT 前原期限已过，必须精确返回 BudgetExceeded，不接受 busy/interrupted 作为等价成功，不将调度等待称为 SQLite 重试。生产连接继续 busy_timeout=0，未修改生产时钟或期限。

失败证据：b355ec80 CI 37888266414、macOS ARM job 113683090046，admission 102.5 µs、before_select 248.949 ms、select 2.667 µs、total 248.951625 ms，实际结果 BudgetExceeded。旧断言把查询前延迟误报为续期。此修复只修正该测试的归因，不解释另两项原生查询超时，不关闭完整 CI、长期运行或监督恢复父门禁。

验证须覆盖实际过期分支与实际未过期锁竞争分支。到期拒绝不是成功授权；不能放宽期限、降低真实输入规模或忽略平台失败以取得绿色。

本机结果：恢复原无条件耗时断言并提供250 ms准备阶段，目标测试真实失败（exit101）；修正后100/250 ms两轮分别返回DatabaseBusy/BudgetExceeded。归属组9通过、1忽略；Store工作区全目标400通过、12忽略，包含既有未提交reader_observation测试，不是精确commit全workspace资格。Store Clippy、CI相同十个项目包格式检查、diff检查通过；vendored副本不修改。四个原错误层级的Scenario标题已修正并经真实delta解析器识别；严格OpenSpec仍有六条既有长Requirement警告，没有宣称通过。证据见`docs/benchmarks/terminal_busy_scheduler_2026_10_09/`。

当前1f4074fb CI37901867819另有macOS ARM HTML撤权测试与Intel历史兼容性测试返回BudgetExceeded；Intel日志记录terminal_ownership_sql约79.9 ms、整个1000 ms历史请求约89.1 ms，独立终态授权窗口耗尽。本项测试修正不解释或关闭这些失败，不放宽生产窗口，不仅因下一轮绿色就删去原失败。Windows台式机正式200k仍300秒超时；独立900秒诊断完成526.891秒，创建145.839、扫描280.626、回收95.313秒，6/6正确性检查。诊断不替代正式门禁，原生命周期及平台父任务不勾选。

后续实际修复：控制库的SQLite配置准备与回调安装耗尽原窗口时，在进入消费者前精确返回BudgetExceeded，并仍清理进度回调、还原busy_timeout。没有刷新生产期限。真实SQLite authorizer延迟配置准备的回归测试在旧行为下失败（expired consumer was executed），修复后的Store全目标401通过、12忽略；Clippy通过。该本机结果包含既有未提交测试，并非完整精确提交资格，证据为control_admission_*文件。

严格OpenSpec现已通过：四个Scenario标题恢复正确层级，六条原有完整约束逐字保留在对应验收Scenario内，未删除或放宽要求。早先六条警告记录保留为历史结果，约束保全检查见spec_constraint_preservation.json。

1f4074fb CI最终23项中21通过、2失败，两项macOS失败仍未关闭，五个原生只读包任务均通过。Windows台式机同E卷、仓库外目录对照已结束：stdio18/18、HTTP14/14、20k6/6，200k仍300秒超时；创建阶段180.656秒，随后进入索引，未取得完整200k报告。路径改变只是输入对照，不是代码优化收益，详见windows_desktop_1f4074fb_2026_10_09/standalone/。长期运行、监督恢复及平台父门禁保持未完成。

修复提交efafb1bf已正常推送到main，并在Windows台式机E:\\workspaces\\workspace-loong10k\\diskgraph快进更新。原生Store全目标413通过、12忽略，Clippy通过，测试前后受控源码干净。重建三个release程序后，stdio18/18、HTTP14/14（60秒、1827次请求、p95约32ms）、20k6/6（扫描38.343秒、32次查询、4并发客户端、p95 106.046ms）通过。首次包装器未绑定stdio/http的DISKGRAPH_ACCEPT_BIN_DIR而失败，修正调用配置后使用相同release摘要验证；失败receipt保留。该批证据见windows_store_efafb1bf_2026_10_09，不宣称性能改善或200k通过。

实机只读环境观察：E盘映射Samsung 990 PRO NVMe、NTFS，剩余约7.45GB；Defender未运行。容量较低仅为相关线索，未证明超时因果，未删除用户数据或改变安全软件配置。efafb1bf CI37908889161尚未终态；Windows与两种Linux只读包任务已通过，不能替代完整CI及台式机200k门禁。为保留当前提交完整验收，不用追加证据提交取消该运行；证据待终态后随下一次授权范围内提交交付。

后续同一CI的Linux/macOS全部六个Rust测试任务以及五个原生只读包任务已经通过，Windows stable/MSRV全workspace测试仍实际运行。ARM stable原始job日志明确包含此前HTML撤权、导入volume/provider历史隔离两项测试通过；一次绿色不用于解释历史79.9ms终态窗口超时的根因。原始日志与绑定见macos_terminal_efafb1bf_2026_10_09。当前Windows CI包200k6/6，创建24.743秒、扫描74.509秒、回收13.416秒、查询p95 32.022ms；仍不替代台式机失败，见windows_package_efafb1bf_2026_10_09。

本机定向测试仅取得配置失败证据：首次CLI错误使用--lib，随后显式旧worker环境触发macOS既定Unsupported策略。只读核实固定active.json不存在；本机没有受保护worker安装，未安装或绕过策略，不能把该失败分类为授权/历史行为失败，也不能计为通过。Windows遗留夹具的只读盘点未开始，连接器在线健康观测与项目执行入口unknown_project相矛盾，重新绑定checkout也未恢复执行；没有据此重派此前验收或删除目录。此连接问题不关闭性能门禁。

CI37908889161现已终态：23项中22通过、1失败，Windows stable的timely_terminal_callback_does_not_pay_for_post_callback_observation返回BudgetExceeded（Engine 729通过、1失败、7忽略）；Windows MSRV和五个原生只读包通过。此前“运行中”文字保留为历史观察。完整原日志、终态元数据与摘要绑定见windows_terminal_callback_efafb1bf_2026_10_09；当前证据未确定回调前/后SQL或取锁哪个阶段失败，不刷新50ms窗口、不放宽断言、不将下次绿色当作根因解释。新增原有调试观测器的四个固定阶段标签，保留原错误和执行顺序，不输出主体或路径。

本轮远程恢复检查：台式机和MacBook两个插件的Runner清单均只返回device-00a75242b0f048e2（aarch64-apple-darwin），此前Windows client在两边均不可见，原项目执行返回unknown_project/not_started。新的词法计划回归尚未执行，不记录红灯或修复通过；保留Windows两处测试修改，不在Mac冒充Windows验收。
