# 扫描预算累计溢出拒绝

状态：实施中。沿用RT-02的预算上限，不调整默认值、期限或发布规则。

累计nodes加1、staged_bytes加实际成本时，u64溢出必须返回对应NodeLimit/StagingLimit，不能以饱和值等于u64::MAX上限作为准入理由。保持原节点优先的判定次序；报告计数仍可饱和为u64::MAX，本次溢出必须返回Stop，调用方收到Stop必须停止；BudgetUsage不新增永久停止锁存状态。精确等于可表示上限继续允许，下一次实际非零累计拒绝。

验收覆盖节点溢出、字节溢出、精确上限及节点优先。此缺陷需极端配置/累计值，不宣称已有现实扫描绕过实例或CPU时限问题根因。

当前验证：旧目标2失败/1通过，修复后macOS Core完整lib167通过/0失败，Core all-target Clippy通过。Linux ARM64 Core完整lib167通过/0失败；独立代码APPROVE、架构CLEAR。Windows原生仍待CI，不声明三平台通过。

当前生产调用仅scan_execution，在收到Stop后立即返回错误并禁止发布；本轮未新增持久锁存状态。
