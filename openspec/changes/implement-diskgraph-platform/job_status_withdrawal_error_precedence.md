# 任务状态读取的撤权与能力超时优先级

延续逐请求授权与 250ms 数据库终检、50ms 能力回调约束。Windows 两种 Rust 的 37253b8d 原生测试均确认：已提交撤权再恢复在任务状态入口被能力回调超时提前覆盖为 BudgetExceeded；既有原撤权见证应报告 PermissionDenied，无法确认通知但授权代次变更应报告 Conflict。

验收：在同一原请求期限仍有效时，读取前验证原撤权见证，再决定能否进入消费者。已撤权/代次变更优先拒绝；未撤权但能力回调超 50ms 仍返回 BudgetExceeded，绝不执行消费者或延长请求期限。原整次期限已到期则立即拒绝，不额外读取。纯能力拒绝仍直接 PermissionDenied。准备响应后的现有原见证与数据库终检保持。

本机确定性 60ms 回调延迟加实际撤权/恢复的回归先失败 0/1（BudgetExceeded 而非本机保守 Conflict）。平台权限语义与固定窗口不删减；后续补充目标和原生验收记录，未完成前不勾选父项。

实现后本机 MCP 管理 9/0、原 MCP 状态锁竞争期限 1/0、CLI 管理 5/0、Engine 全平台 AST 6/0、CLI AST 2/0；三包 all-target Clippy、fmt、OpenSpec 严格校验通过。已提交撤权再恢复的确定性 60ms 回调延迟得到原撤权拒绝；无撤权但同样迟到的 Allow 拒绝且消费者未运行，短原期限到期不续期。迟到能力仅可在 min(原期限, 250ms 数据库窗口) 中核验拒绝原因；不是新的业务许可。

证据 docs/benchmarks/job_status_withdrawal_precedence_2026_10_12/receipt.json。Windows 修复后的原生结果仍待 CI；37253b8d 的失败原样保存，不以本机保守 Conflict 替代 Windows 原生撤权 PermissionDenied。
