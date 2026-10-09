# 长连接主体存活窄读

沿用请求真实主体、token 能力与实时授权交集合同。长连接周期观察应只读取当前主体和候选权限的当前 epoch grant，以及这些 grant 指向的 scope；不得重建所有主体策略或遍历无关 scope。保留管理范围无注册 scope 的既有语义、未发布/撤销/版本更新拒绝、真实提交撤权可见性、原 50 ms 观察期限与失败关闭，不缓存允许结果。

验收：实际 SQLite progress_handler 计量，在增加 2000 个其他主体 grant 后单次有效观察工作少于 1000 VM 步；覆盖独立连接撤权、恢复、scope 撤销、管理授权、空/不匹配 token 权限、策略未发布/撤销/换代和 SQL 中断。先用原逻辑取得性能 RED，再实现窄读。该子项不替代 Windows 默认并发、真实 SSE 撤权、恢复监督或全平台生产验收。

实现：Engine 的长连接观察接入主体/权限主键前缀查询，仅对候选 grant 的 scope 点查；每次 SQL 仍读取持久策略及当前 epoch，不缓存授权结果。原 control_until/with_read_deadline 及 50 ms 期限保持不变，管理范围沿用已有无注册记录的授权语义。

实际 TDD：macOS 原逻辑在 2000 个无关主体 grant 下为 12059 VM 步，性能测试 RED，语义测试通过。实现后增加 2000 个无关 scope 的更大夹具为 54 步；macOS 与 Windows 两项目标测试均通过，Windows 同样为 54 步。VM 指令不等于物理 IO、端到端耗时或 RSS，两个阶段的夹具大小差别明确保留，不称为严格同输入时间基准。

回归：macOS Store 364/0/8，Windows Store 376/0/8；Store/Engine 全目标严格 Clippy、定向 rustfmt、diff 检查通过。Windows 原期限默认并发 MCP 229/2/0，仍有一次新鲜归属 SQL 63.497 ms 超限，以及 legacy 会话 404 而非 202。上一轮归属编译复用后的 CLI 单元 88/0、全部 CLI 集成通过，MCP 单元 228/3；另一次诊断为 228/3 且失败项目不同。失败波动不能证明本优化修复了 SSE，也不能替代端到端稳定性验收。原始程序/源码/日志摘要见 windows_live_identity_narrow_2026_10_09.json 和 windows_frontends_ownership_cache_82e692d4_2026_10_09.json。

状态：本窄读实现及上述局部验证完成，实际全平台生产门禁保持开放。CI 37872537642 绑定旧 82e692d4，不含此优化；本机历史集成仍缺受信宿主部署，未据 Store 通过声称已覆盖。

后续定位：沿现有显式 debug 授权诊断，为长连接存活观察增加固定 stream_identity_lock/stream_identity_sql 阶段标签。仅原 BudgetExceeded/busy/interrupted 才输出无身份与路径的耗时；不改变错误、原期限或连接关闭行为。须用实际默认并发 Windows 结果区分控制锁与 SQL 耗尽，不能由 404 推定撤权或缓存问题。

阶段诊断首次 Windows MCP 默认并发 231/0，输出中包含刻意持锁回归引发的 lock 超限和一次 SQL 75.369 ms，不能将这些无请求关联的固定标签归因于未复现的 legacy 案。实际握手顺序缺陷另用独立真实 socket RED/GREEN 修复，详见 sse_handshake_live_authority.md；全平台稳定性父项仍开放。
