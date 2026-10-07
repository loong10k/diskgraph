# 历史终检期限观察：尚未关闭

提交 `3f32b2864708ae306a257a1739f99dc1ff5e91a3` 的 CI `37616988613`，Linux stable job `112777608800` 的完整 workspace 中，`history_compatibility_matrix::node_quality_and_type_replacement_matrix_keeps_unknown_bytes_null` 在 compare 的 unwrap 返回 `Business(BudgetExceeded)`；同一矩阵其余九项通过。Linux MSRV job `112777608811` 的完整 Test 步骤通过。

当前无法据此定位到控制锁争用、SQL 授权观察窗口或原数据期限；不得将调度假设视为根因，不增加固定生产期限，不以重试掩盖原失败。macOS 本机单用例在原生宿主构造时返回 Unsupported，不是该行为的有效复现。

Linux CI 增加同一原生用例的独立执行并保留日志，以区分隔离与并发观察。原完整 workspace 仍执行且失败仍判失败。隔离证据不能替代全量通过，生产门禁仍未完成。

## 同轮归属连接复用候选

双侧终检每轮用一个新建只读连接逐项查询；编码后重新打开另一新连接，不复用消费者连接、不建立冻结归属的读事务、不缓存授权。双侧同轮原 50 ms 期限和末段撤权优先规则保持不变。

真实连接打开计数回归：旧实现编码前后共打开 4 个，预期每轮一次而失败；候选编码前后共 2 个而通过。隔离合法发布夹具的撤权优先、控制锁持有期间拒绝回归通过；两个不同合法 revision 的双侧成功与第二侧归属不匹配拒权回归通过。Engine 源码门禁 6/0、all-target Clippy 通过。独立代码与架构复核未发现阻断问题；双侧仍为顺序观察，不宣称原子授权快照。它是减少连接成本的验证，不是 Linux stable 超时根因已关闭的证据；原生 CI 尚待完成。
