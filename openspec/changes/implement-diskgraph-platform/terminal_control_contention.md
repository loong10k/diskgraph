# 关系与历史查询末段锁竞争

沿用 implement-diskgraph-platform / bounded-queries。两条真实查询在完成消费者后通过阻塞 control_store 获取终检 guard；另一线程可使原请求等待到持锁者释放，超出数据期限。隔离合法 Store revision 测试的旧实现两路径均出现 200ms 观察 Timeout；观察期限只属于测试，失败后释放原 guard 并实际 join。

改为 try_control_store：竞争立即 BudgetExceeded，锁中毒仍为原 Poisoned。没有获准读取实时授权时不返回完整或部分数据；不把竞争当作撤权。取得原 guard 后权限、实际 revision/server/scope 归属及编码后复检顺序不变，不刷新数据期限。短暂竞争现在也可能拒绝查询，这是明确的失败关闭行为。

新锁竞争/无竞争两路径测试、两路径读后真实 scope 撤销、已有 expired-envelope 实时授权回归均通过；Engine源码门禁6/0及all-target Clippy通过。两个独立审查批准。原真实扫描guard等待测试保留扫描前置，结果改为 BudgetExceeded；本机因原宿主部署资格不足在构造阶段 Unsupported，不能算该行为通过，须原生CI补验。

这项修复不证明 SQLite等待、同步Authorizer回调的全部期限，不解释无竞争Linux查询性能回退，也不关闭监督或整体生产门禁。
