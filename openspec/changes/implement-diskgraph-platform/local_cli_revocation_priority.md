# 本地 CLI 已知整体撤销的拒权优先级

## 已复现问题与验收行为

368e6691 的 CI38210169373 已终态失败，18/23 job 通过。Windows MSRV job114682339208 的 Engine801/0/8、MCP237/0通过，CLI flow15/1；唯一失败为 policy_revocation_denies_everything_until_republish：策略整体撤销后 children 返回 budget_exceeded/7，而原断言要求 permission_denied/3。原日志见 `docs/benchmarks/ci_368e6691_terminal_2026_10_12/`。不能把其视为此前 scope 显示用例再次失败。

原本地入口已持有 LocalIdentity 从持久策略加载的 PolicyAuthorizer，却在 dispatch 后先解析最新 revision，随后才检查能力。整体撤销已是该请求的已知负向事实，不需要再次读取图库才能拒绝。恢复既有 CLI 撤销验收契约：已知整体策略撤销时，已启用的快照读取/展示命令直接返回 PermissionDenied，输出缓冲保持为空；未撤销请求必须继续原完整授权、实际归属、实时撤权及原查询期限。policy publish/revoke、安装和既有关闭工具的分发保持原路径，不阻断可信本地恢复入口。

## 实现与验证

新增 PolicyAuthorizer::is_revoked 只读取当前策略快照的撤销字段；false 绝不是授权证明。CLI entry 接入 dispatch_local，仅整体撤销时拒绝读取命令；其他情况委托原 dispatch。它不调用额外宿主回调、不做SQL、不延长预算、不放宽资源授权。普通自定义 Authorizer 的真实回调与终检路径不变，远程 MCP/FFI 不使用本地兼容入口。

先用原分发逻辑建立本地入口，新增真实隔离图库回归得到 RED0/1：已撤销快照与过期原期限返回 BudgetExceeded；修复后相关11项通过。正向 wire/Unicode 和未授权主体原断言改为调用实际本地分发入口，未删除或放宽断言。新增未撤销+过期期限回归仍必须 BudgetExceeded，证明没有续期或允许捷径。Core178项、Core/CLI all-target Clippy、CLI源码门禁及相关文件fmt通过。

本机回归使用合成元数据和真实 staging/revision，不冒充物理扫描、Windows CLI进程或全workspace验收；原 Windows MSRV CLI flow 用例保持原断言，待新CI执行。原生 macOS stable 的终检读取超时、修正后的 Linux guardian 正常扫描/stdio MCP、其他平台独立服务及最终同SHA验收继续开放。未改变数据库250ms、能力50ms或整次查询预算。

候选源码、RED/GREEN和检查日志摘要见 `docs/benchmarks/local_revoked_policy_368e6691_2026_10_12/receipt.json`。

45a17286 的 CI38215593380 已终态20/23；Windows stable/job114699450569 与 MSRV/job114699450581 均成功，原 CLI flow policy_revocation_denies_everything_until_republish 在两份真实日志均实际执行并通过，原 scope 短预算资格也分别实际通过两次。此证据完成本次已知本地快照撤销优先级的 Windows 回归，不声称关闭其余延迟来源、Windows桌面服务或完整生产父项。

三项Linux作业仅在MCP信封断言处失败；实际返回原scope/revision、ok=true、data.node.files=3。8ce80fcf修正后正常EOF与原容量退休尚待新CI，不以成功查询替代退休。原始job状态、Windows完整日志及Linux x86/ARM日志摘要见 `docs/benchmarks/ci_45a17286_terminal_2026_10_12/`。
