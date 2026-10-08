# 已知撤权在末段 SQL 超时后的优先级

## 原生失败与修复边界

原生 CI 37776446882 对应 d8d3995f4f5cc1c2bbf09263814d18b322900f79，最终 19 成功、4 失败。Windows Rust 1.97 作业 113308401977 的 Engine 为 681 通过、1 失败、4 忽略；失败为 `history_and_relation_remember_withdrawal_in_capability_observations`，实际 BudgetExceeded，预期 PermissionDenied。日志的 relation_terminal_control 为 60005 微秒。原始 gzip 日志、作业终态与摘要保存在 `docs/benchmarks/ci_d8d3995f_terminal_2026_10_08/`。此证据不能说明 macOS Intel explain 的失败原因。

既有 SC-04 要求原连接已知、已成功持久撤销的精确请求依赖，即使随后重授或观察超时，也应拒权。旧关系/历史路径只在 SQL 观察末尾检查见证，因此 SQL 提前超时会遮蔽已经存在的负向事实。

修复在整个末段授权返回 BudgetExceeded 后，非阻塞尝试原控制连接，仅在纯内存原生见证确认 Withdrawn 时返回 PermissionDenied。不添加 SQL，不等待控制锁，不延长任一窗口，不缓存 Allow，不让失败结果变为成功；未知、失效和锁繁忙均保留预算错误。普通成功路径继续执行全部实时授权、独立新鲜 revision 归属与编码前后检查。关系/历史锁争用立即拒绝的原契约不变。

## 验证设计与限制

新增测试在真实已创建的 50ms 末段 SQL 窗口中等待其过期，关系/历史两条产品路径分别验证：真实能力回调撤销并重授后拒权；未发生撤权时仍为预算错误；均不得进入编码。Windows 测试强制实际原生 watch 存在，不能退化为未知能力分支。

本机 macOS 的默认 SQLite VFS 没有已资格的原生撤权 watch，因此修复前这两个新增测试通过的是未知能力分支，不属于红灯证明。该输出明确命名为 pre_fix_macos_unknown_watch.log.gz；真实红灯来自上述 Windows 原生 CI。修复后 Windows 正向分支仍须新提交原生 CI 验证，本机通过不能替代。

本机扩大到 terminal_ 过滤的回归为 44 通过、17 失败，其中 15 项在缺失受信原生扫描部署材料时返回 Unsupported，另 2 项为授权预算错误。保留原失败日志；不得称完整本机回归通过，也不能用后续隔离运行抹去此轮负载下失败。

本项不关闭整体查询稳定性、产品监督恢复、平台性能或生产就绪门禁，未勾选父任务。既有 Windows 200k 全流程 300 秒门槛继续保持失败；仍缺 macOS 真实 FileProvider 与最终同 SHA 平台验收。

本机目标组结果：history_relation_withdrawal_tests 6/6、terminal_capability_tests 14/14；Engine all-target Clippy 和项目指定10包格式检查通过。扩大回归中两个预算失败的原测试随后分别隔离1/1通过，只说明隔离行为，不消除并发失败。源文件摘要、压缩输出与完整失败均在 `docs/benchmarks/terminal_withdrawal_expiry_2026_10_08/`，同 SHA 原生全量仍待验证。

用户已将 Windows 原生验证交给自己的台式机，并授权当前任务通过 WebCodex-台式机访问对应仓库、更新代码及运行验证。执行清单见 `windows_desktop_acceptance_tasks.md`；当前连接未恢复，macOS/Linux 与平台无关工作继续，不把连接问题记为功能通过。
