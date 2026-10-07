# 历史终检期限观察：尚未关闭

提交 `3f32b2864708ae306a257a1739f99dc1ff5e91a3` 的 CI `37616988613`，Linux stable job `112777608800` 的完整 workspace 中，`history_compatibility_matrix::node_quality_and_type_replacement_matrix_keeps_unknown_bytes_null` 在 compare 的 unwrap 返回 `Business(BudgetExceeded)`；同一矩阵其余九项通过。Linux MSRV job `112777608811` 的完整 Test 步骤通过。

当前无法据此定位到控制锁争用、SQL 授权观察窗口或原数据期限；不得将调度假设视为根因，不增加固定生产期限，不以重试掩盖原失败。macOS 本机单用例在原生宿主构造时返回 Unsupported，不是该行为的有效复现。

Linux CI 增加同一原生用例的独立执行并保留日志，以区分隔离与并发观察。原完整 workspace 仍执行且失败仍判失败。隔离证据不能替代全量通过，生产门禁仍未完成。
