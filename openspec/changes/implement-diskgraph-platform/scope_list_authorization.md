# scope 列表授权终态与锁边界

沿用 SC-04 / 请求能力与实时持久授权交集，保留既有管理回退和 wire 结构。

验收：scope能力回调在控制锁外执行；所有scope回调结束后，最终返回前对当前持久授权求交集。管理员回退不得使用回调之前缓存的允许状态。后续scope回调撤销先前scope的grant时不得返回先前scope。独立连接撤权使用隔离夹具；不冻结独立连接，也不声称检查之后永远不会发生撤权。可信无策略兼容和管理员仅持元数据admin授权回退保留，损坏数据仍失败关闭。此接口未新增严格响应/时间预算，不关闭全平台任务。

所有回调结束后和最终SQL完成后检查固定token到期时间；迟到Allowed不得提交。无到期信息的可信本机能力不新增TTL。固定expiry、实际越过到期的回归严格PermissionDenied；不保证同步能力回调硬抢占。

本机验证：旧路径3项真实RED、固定expiry旧路径另1项RED；最终5/0（含admin-only与无策略正控），source_layout6/0、fmt/Clippy通过、双路APPROVE/CLEAR。完整Engine545/98/13，98项均Unsupported。新增决定列表O(n)，未实测性能收益；候选scope来自回调前注册表观察，不宣称原子快照或最后观察后撤权线性化。同步回调、锁等待与大列表严格预算仍未关闭。

## 注册入口的双锁外能力观察

register_scope保留初次管理授权与规范根解析；第二次能力决定在graph/control双锁外取得，随后仍按graph→control顺序复核当前持久ScopeAdmin，原注册写守卫固定expiry与deadline保持。回调重入控制读取两次都必须成功；第二次回调独立撤销admin grant，旧Allowed不得出生新scope。此项未给图/控制锁等待增加硬抢占，不替代监督服务和平台验收。

注册锁分离验证：有限重入旧路径真实RED→GREEN，scope授权组7/0；source_layout6/0、fmt/Clippy与双路APPROVE/CLEAR。完整Engine547/98/13，98项均Unsupported。兼容边界：第二次回调先于双锁获取错误执行；锁外能力的独立动态撤销不保证与写提交原子一致，持久管理员与固定expiry仍由原事务守卫核验。
