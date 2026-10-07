# Windows 原身份通知完成合同

## 问题与验收

923b6d8 原生 Windows Rust 1.97 job 112696120094 的完整测试在 Git 根清理确认失败，原 NTSTATUS 0xc000000d、Win32 87、ID 长度 8。附加 ID 查询不能产生通知型清理的成功证明，却提前中断原 REMOVE 通知观察。

通知型确认仅在删除前原订阅、原 held parent/child 身份、完整合法原 REMOVE 页及原 I/O 完成均成立时返回 true。尚无页或完整但不匹配的页返回 Pending；保持原 owner、订阅与容量。无效/丢失通知、身份变化和原 I/O 错误仍拒绝。独立 WindowsGitDeletionWitness 保留原正控与未知错误拒绝合同；87、权限错误或路径消失绝不是删除证明。

清理只在同一 ProbeBudget 的原绝对期限与取消约束内轮询；不在每次 Pending 重建预算。到期、取消或错误时保留原恢复 payload。测试必须覆盖实际外部原文件句柄阻止完成、Pending 不释放、关闭外部句柄后原匹配通知完成、陌生同名目录未被删除；独立 ID witness 的 present 正控保留。

## 验证状态

原完整 CI RED 已取得。更新后的通知型 Pending 回归及产品修复需要新提交 Windows 原生 CI，macOS check 不能证明此能力；验收尚未完成。原 SSOT 中禁止放宽生产 ID 查询的要求仍适用，本变更不修改 ID witness，而删除通知型确认中不能授予成功的附加 ID 查询。

## 恢复公平性

同步 cleanup 在一个原 ProbeBudget 内等待通知；有界 cleanup_until 每次推进到 Pending 即返回 Ok(false)，保留原帧及订阅，把执行机会交给后续槽。不得按错误字符串推断 Pending。同步等待使用 min(1ms, 原期限剩余量)。真实双槽公平性覆盖首槽外部句柄阻碍、次槽可完成、首槽仍占用以及解除后完成；原生执行证据见下文。

## 双槽原生证据（2026-10-07）

提交 bfee4f2005aaf2630f1e4c9fd39beb7c0186252b、CI 37597292546 的 Windows stable job 112712926763 与 Rust 1.97.0 job 112712926669 均已执行精确双槽测试：各 1 passed、0 failed、0 ignored、0.02 秒，并输出 DG_PENDING_FIRST_ORIGINAL_SECOND_RETIRED=1。原日志与摘要见 docs/benchmarks/windows_pending_pool_bfee4f2/receipt.json。

该证据覆盖真实首槽外部原句柄保留、第二槽退休、第一槽容量保持及解除阻碍后的原恢复完成；不证明任意目录遍历阶段严格时间片或原生 OS 调用硬期限。完整 Windows suite、其他通知回归与全平台生产验收仍待完成，不据此勾选总任务。

## 完整回归 RED 与断言修正

同一提交的 Windows Rust 1.97.0 完整 Test 在 engine lib 得到 572 passed、2 failed、3 ignored；两个失败是 cleanup_cursor_retains_current_child_across_open_failure_and_pending_deletion 和 cleanup_mark_requires_current_identity_and_live_budget_before_mutation。原生返回严格 Pending Ok(false)，两处遗留断言仍调用 unwrap_err 并期望错误 5。完整原日志及失败回执保存在 docs/benchmarks/windows_cursor_pending_bfee4f2_red/。

候选修正仅改测试和 CI：持有原句柄时严格要求 false，禁止游标跳过当前 child 的断言保留；关闭后沿同一 ProbeBudget 等待原通知，未知错误和到期直接失败。mark 案增加原文件实际消失与 foreign sentinel 未变校验。两案新增前置精确 native gate，分别要求 1/0/0 和原成功标记；生产通知及独立 ID witness 代码不改变。新原生验收尚待完成，不将此 RED 改记为成功。


## 逐文件诊断的执行成本

测试构建的 REMOVE 页及 post-mark 诊断仅在显式环境变量 `DISKGRAPH_TRACE_WINDOWS_CLEANUP=1` 时输出；按测试线程缓存开关，禁用时不进行额外诊断用 FILE_STANDARD_INFO 查询。真实通知解析、匹配、删除 seal、容量记账和竞态 hook 始终执行，生产构建不读取该变量。成功/失败验收标记及原断言不受影响。

PowerShell 排障：`$env:DISKGRAPH_TRACE_WINDOWS_CLEANUP='1'` 后执行原精确测试。每个新测试进程在第一次使用诊断时固定该值。默认关闭只减少诊断成本，尚未测量墙钟收益，也不证明任何生产授权或恢复门禁通过。

English: Per-file REMOVE and post-mark tracing is opt-in in test builds via `DISKGRAPH_TRACE_WINDOWS_CLEANUP=1`. The disabled path avoids the additional diagnostic metadata query. Authorization, identity seals, notifications, accounting, race hooks and acceptance assertions still execute. No timing improvement is claimed until native measurements exist.
